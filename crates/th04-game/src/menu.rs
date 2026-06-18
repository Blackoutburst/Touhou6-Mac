//! Title screen + menu state machine, driven by the same 60 Hz `update`
//! closure that runs the stage. Flow:
//!
//! ```text
//! Title ─Z─▶ Main ─┬─START────▶ Char ▶ Shot ▶ Rank ─────────▶ Play (full run)
//!                  ├─PRACTICE─▶ Char ▶ Shot ▶ Rank ▶ Stage ─▶ Play (one stage)
//!                  ├─EXTRA────▶ Char ▶ Shot ▶ Rank ─────────▶ Play (ST06)
//!                  └─QUIT─────▶ exit
//!
//! Play ─clear─▶ (full run: next stage, carrying score/lives) ─… last─▶ Result
//!      ─miss out of lives─▶ Result ─Z─▶ Title
//! ```
//!
//! All stages' textures are fixed up front (the engine owns them for the whole
//! loop — see [`crate::build_all_stages`]), so the menu can offer any stage and
//! the sim is built on demand from the chosen stage's [`crate::StageAssets`].

use th04_formats::sim::StageSim;
use th06_engine::{DrawCmd, Frame, Input, Key};

use crate::font::{draw_text, draw_text_centered, text_width};
use crate::{boss_for, draw_frame, map_input, StageAssets};

const SCREEN_W: f32 = 640.0;
/// `ST05` (stage 6) is the last stage of a full run; `ST06` is the Extra stage,
/// only reachable via its own menu entry / practice.
const LAST_NORMAL_STAGE: usize = 5;

#[derive(Clone, Copy, PartialEq)]
enum MainAction {
    Start,
    Practice,
    Extra,
    Option,
    Disabled,
    Quit,
}

/// Player-configurable settings (the OPTION screen), applied to fresh runs.
#[derive(Clone, Copy)]
struct Config {
    start_lives: i32,
    start_bombs: i32,
}
impl Default for Config {
    fn default() -> Self {
        Config { start_lives: 2, start_bombs: 3 }
    }
}
/// Inclusive adjust ranges for the OPTION settings.
const LIVES_RANGE: (i32, i32) = (1, 5);
const BOMBS_RANGE: (i32, i32) = (0, 3);

/// What kind of run is being played (controls what happens after a stage clear).
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// Full game: clearing a stage advances to the next, carrying score/lives.
    Full,
    /// A single stage (practice / extra): clearing it ends the run.
    Single,
}

const MAIN_ENTRIES: &[(&str, MainAction)] = &[
    ("START", MainAction::Start),
    ("PRACTICE START", MainAction::Practice),
    ("EXTRA START", MainAction::Extra),
    ("MUSIC ROOM", MainAction::Disabled),
    ("OPTION", MainAction::Option),
    ("QUIT", MainAction::Quit),
];

const CHARACTERS: [&str; 2] = ["REIMU HAKUREI", "MARISA KIRISAME"];
const SHOTS: [&str; 2] = ["TYPE A", "TYPE B"];
const RANKS: [&str; 4] = ["EASY", "NORMAL", "HARD", "LUNATIC"];

enum Screen {
    Title,
    Main(usize),
    Char(usize),
    Shot(usize),
    Rank(usize),
    /// Practice stage picker (cursor over stages 1..=6, i.e. indices 0..=5).
    StageSelect(usize),
    /// Settings editor (cursor over the OPTION rows).
    Option(usize),
    Playing {
        sim: Box<StageSim>,
        stage: usize,
        mode: Mode,
    },
    /// Post-run splash; `t` counts it down before returning to the title.
    Result {
        cleared: bool,
        all_clear: bool,
        score: i64,
        t: u32,
    },
}

pub struct MenuApp {
    screen: Screen,
    stages: Vec<StageAssets>,
    /// Title image: texture index + draw size (None → a text-only title).
    title_tex: Option<(usize, f32, f32)>,
    /// Which main-menu action opened the character→shot→rank sequence.
    pending: MainAction,
    // Selection carried across the character → shot → rank → stage screens.
    character: usize,
    shot: usize,
    rank: usize,
    config: Config,
    high_score: i64,
    blink: u32,
}

impl MenuApp {
    pub fn new(stages: Vec<StageAssets>, title_tex: Option<(usize, f32, f32)>) -> Self {
        MenuApp {
            screen: Screen::Title,
            stages,
            title_tex,
            pending: MainAction::Start,
            character: 0,
            shot: 0,
            rank: 1, // default Normal
            config: Config::default(),
            high_score: 0,
            blink: 0,
        }
    }

    fn shot_type(&self) -> u8 {
        (self.character as u8) * 2 + self.shot as u8
    }

    /// Build a fresh sim for `stage` with the current character + difficulty.
    fn build_sim(&self, stage: usize) -> StageSim {
        let a = &self.stages[stage];
        let shot_type = self.shot_type();
        let mut sim = StageSim::new(a.std.clone(), shot_type, boss_for(&a.name, shot_type));
        sim.set_rank(self.rank as u8);
        sim
    }

    /// Begin a run at `stage` in `mode`, seeding the player from the OPTION
    /// config (chained stages instead carry over via `StageSim::restore`).
    fn start(&self, stage: usize, mode: Mode) -> Screen {
        let mut sim = self.build_sim(stage);
        sim.player.lives = self.config.start_lives;
        sim.player.bombs = self.config.start_bombs;
        Screen::Playing { sim: Box::new(sim), stage, mode }
    }

    /// What to do once the player confirms difficulty, based on `pending`.
    fn after_rank(&self) -> Screen {
        match self.pending {
            MainAction::Practice => Screen::StageSelect(0),
            MainAction::Extra => self.start(6, Mode::Single), // ST06
            _ => self.start(0, Mode::Full),                   // START
        }
    }

    /// Advance one frame; returns the frame to draw (and whether to quit).
    pub fn update(&mut self, inp: &Input) -> Frame {
        self.blink = self.blink.wrapping_add(1);
        let (pu, pd) = (inp.pressed(Key::Up), inp.pressed(Key::Down));
        let (pl, pr) = (inp.pressed(Key::Left), inp.pressed(Key::Right));
        // List screens treat left/right as up/down too (forgiving); the OPTION
        // screen uses the pure left/right to adjust values.
        let up = pu || pl;
        let down = pd || pr;
        let confirm = inp.pressed(Key::Shoot) || inp.pressed(Key::Enter);
        let back = inp.pressed(Key::Pause) || inp.pressed(Key::Bomb);

        let mut quit = false;
        let mut cmds = Vec::new();

        // Own the current screen so we can freely call &self/&mut self helpers
        // and reassign the next screen without borrow conflicts.
        let screen = std::mem::replace(&mut self.screen, Screen::Title);
        self.screen = match screen {
            Screen::Title => {
                self.draw_backdrop(&mut cmds, false);
                if (self.blink / 30) % 2 == 0 {
                    draw_text_centered(&mut cmds, SCREEN_W / 2.0, 420.0, "PRESS Z", 4.0, [1.0; 4]);
                }
                self.draw_hiscore(&mut cmds, 458.0);
                if confirm {
                    Screen::Main(0)
                } else {
                    quit = back;
                    Screen::Title
                }
            }
            Screen::Main(cursor) => {
                let cursor = step_cursor(cursor, MAIN_ENTRIES.len(), up, down);
                self.draw_backdrop(&mut cmds, true);
                self.draw_hiscore(&mut cmds, 30.0);
                let entries: Vec<(&str, bool)> =
                    MAIN_ENTRIES.iter().map(|(s, a)| (*s, *a != MainAction::Disabled)).collect();
                draw_menu_list(&mut cmds, 210.0, &entries, cursor);
                if confirm {
                    match MAIN_ENTRIES[cursor].1 {
                        MainAction::Quit => {
                            quit = true;
                            Screen::Main(cursor)
                        }
                        MainAction::Disabled => Screen::Main(cursor),
                        MainAction::Option => Screen::Option(0),
                        action => {
                            self.pending = action;
                            Screen::Char(self.character)
                        }
                    }
                } else if back {
                    Screen::Title
                } else {
                    Screen::Main(cursor)
                }
            }
            Screen::Char(cursor) => {
                let cursor = step_cursor(cursor, CHARACTERS.len(), up, down);
                self.draw_backdrop(&mut cmds, true);
                draw_heading(&mut cmds, "SELECT CHARACTER");
                draw_menu_list(&mut cmds, 240.0, &labels(&CHARACTERS), cursor);
                if confirm {
                    self.character = cursor;
                    Screen::Shot(self.shot)
                } else if back {
                    Screen::Main(0)
                } else {
                    Screen::Char(cursor)
                }
            }
            Screen::Shot(cursor) => {
                let cursor = step_cursor(cursor, SHOTS.len(), up, down);
                self.draw_backdrop(&mut cmds, true);
                draw_heading(&mut cmds, "SELECT SHOT TYPE");
                draw_menu_list(&mut cmds, 240.0, &labels(&SHOTS), cursor);
                if confirm {
                    self.shot = cursor;
                    Screen::Rank(self.rank)
                } else if back {
                    Screen::Char(self.character)
                } else {
                    Screen::Shot(cursor)
                }
            }
            Screen::Rank(cursor) => {
                let cursor = step_cursor(cursor, RANKS.len(), up, down);
                self.draw_backdrop(&mut cmds, true);
                draw_heading(&mut cmds, "SELECT DIFFICULTY");
                draw_menu_list(&mut cmds, 210.0, &labels(&RANKS), cursor);
                draw_text_centered(&mut cmds, SCREEN_W / 2.0, 396.0, "PATTERNS ARE NORMAL RANK", 2.0, [0.6, 0.6, 0.7, 1.0]);
                if confirm {
                    self.rank = cursor;
                    self.after_rank()
                } else if back {
                    Screen::Shot(self.shot)
                } else {
                    Screen::Rank(cursor)
                }
            }
            Screen::StageSelect(cursor) => {
                let cursor = step_cursor(cursor, LAST_NORMAL_STAGE + 1, up, down);
                self.draw_backdrop(&mut cmds, true);
                draw_heading(&mut cmds, "SELECT STAGE");
                let names: Vec<String> = (0..=LAST_NORMAL_STAGE).map(|i| format!("STAGE {}", i + 1)).collect();
                let entries: Vec<(&str, bool)> = names.iter().map(|s| (s.as_str(), true)).collect();
                draw_menu_list(&mut cmds, 150.0, &entries, cursor);
                if confirm {
                    self.start(cursor, Mode::Single)
                } else if back {
                    Screen::Rank(self.rank)
                } else {
                    Screen::StageSelect(cursor)
                }
            }
            Screen::Option(cursor) => {
                const ROWS: usize = 3; // START LIVES, START BOMBS, EXIT
                let cursor = step_cursor(cursor, ROWS, pu, pd);
                // Left/right adjusts the focused setting.
                let d = (pr as i32) - (pl as i32);
                if d != 0 {
                    match cursor {
                        0 => self.config.start_lives = (self.config.start_lives + d).clamp(LIVES_RANGE.0, LIVES_RANGE.1),
                        1 => self.config.start_bombs = (self.config.start_bombs + d).clamp(BOMBS_RANGE.0, BOMBS_RANGE.1),
                        _ => {}
                    }
                }
                self.draw_backdrop(&mut cmds, true);
                draw_heading(&mut cmds, "OPTION");
                draw_option_row(&mut cmds, 200.0, "START LIVES", &self.config.start_lives.to_string(), cursor == 0);
                draw_option_row(&mut cmds, 250.0, "START BOMBS", &self.config.start_bombs.to_string(), cursor == 1);
                draw_option_row(&mut cmds, 320.0, "EXIT", "", cursor == 2);
                // EXIT row (or back) returns to the main menu.
                if back || (confirm && cursor == 2) {
                    Screen::Main(0)
                } else {
                    Screen::Option(cursor)
                }
            }
            Screen::Playing { mut sim, stage, mode } => {
                if back {
                    Screen::Title // abandon the run
                } else {
                    sim.step(&map_input(inp));
                    cmds = draw_frame(&sim, &self.stages[stage].dd);
                    if !sim.finished() {
                        Screen::Playing { sim, stage, mode }
                    } else {
                        self.high_score = self.high_score.max(sim.score);
                        if sim.player.gameover {
                            Screen::Result { cleared: false, all_clear: false, score: sim.score, t: 0 }
                        } else if mode == Mode::Full && stage < LAST_NORMAL_STAGE {
                            // Advance to the next stage, carrying the run state.
                            let next = stage + 1;
                            let mut ns = self.build_sim(next);
                            ns.restore(sim.player.lives, sim.player.bombs, sim.player.power, sim.score, sim.extends_awarded);
                            Screen::Playing { sim: Box::new(ns), stage: next, mode }
                        } else {
                            let all_clear = mode == Mode::Full;
                            Screen::Result { cleared: true, all_clear, score: sim.score, t: 0 }
                        }
                    }
                }
            }
            Screen::Result { cleared, all_clear, score, t } => {
                self.draw_backdrop(&mut cmds, true);
                let (msg, col) = if all_clear {
                    ("ALL CLEAR", [1.0, 0.9, 0.4, 1.0])
                } else if cleared {
                    ("STAGE CLEAR", [0.6, 1.0, 0.7, 1.0])
                } else {
                    ("GAME OVER", [1.0, 0.5, 0.5, 1.0])
                };
                draw_text_centered(&mut cmds, SCREEN_W / 2.0, 170.0, msg, 6.0, col);
                draw_text_centered(&mut cmds, SCREEN_W / 2.0, 260.0, &format!("SCORE {}", score), 3.0, [1.0; 4]);
                draw_text_centered(&mut cmds, SCREEN_W / 2.0, 300.0, &format!("HI-SCORE {}", self.high_score), 3.0, [0.9, 0.9, 0.6, 1.0]);
                draw_text_centered(&mut cmds, SCREEN_W / 2.0, 360.0, "PRESS Z", 3.0, [0.8; 4]);
                if confirm || back || t > 600 {
                    Screen::Title
                } else {
                    Screen::Result { cleared, all_clear, score, t: t + 1 }
                }
            }
        };

        Frame { cmds, bg: None, quit }
    }

    /// Title image (or a text logo), optionally darkened for the menu overlay.
    fn draw_backdrop(&self, cmds: &mut Vec<DrawCmd>, dim: bool) {
        cmds.push(fill(0.0, 0.0, SCREEN_W, 480.0, [0.02, 0.02, 0.05, 1.0]));
        match self.title_tex {
            Some((tex, w, h)) => {
                let x = (SCREEN_W - w) / 2.0;
                let y = (480.0 - h) / 2.0;
                cmds.push(DrawCmd { tex, dst: [x, y, w, h], src: [0.0, 0.0, 1.0, 1.0], tint: [1.0; 4], rot: 0.0 });
            }
            None => {
                draw_text_centered(cmds, SCREEN_W / 2.0, 120.0, "TOUHOU 4", 7.0, [1.0, 0.9, 0.5, 1.0]);
                draw_text_centered(cmds, SCREEN_W / 2.0, 190.0, "LOTUS LAND STORY", 4.0, [0.9, 0.8, 1.0, 1.0]);
            }
        }
        if dim {
            cmds.push(fill(0.0, 0.0, SCREEN_W, 480.0, [0.0, 0.0, 0.05, 0.55]));
        }
    }

    fn draw_hiscore(&self, cmds: &mut Vec<DrawCmd>, y: f32) {
        draw_text_centered(cmds, SCREEN_W / 2.0, y, &format!("HI-SCORE {}", self.high_score), 2.0, [0.9, 0.9, 0.6, 1.0]);
    }
}

/// Move a wrapping-free cursor by the pressed direction.
fn step_cursor(cursor: usize, len: usize, up: bool, down: bool) -> usize {
    let mut c = cursor.min(len.saturating_sub(1));
    if up && c > 0 {
        c -= 1;
    }
    if down && c + 1 < len {
        c += 1;
    }
    c
}

/// Turn a slice of labels into `(label, enabled=true)` entries.
fn labels<'a>(items: &'a [&'a str]) -> Vec<(&'a str, bool)> {
    items.iter().map(|s| (*s, true)).collect()
}

/// Solid quad on the white texture (absolute screen coords).
fn fill(x: f32, y: f32, w: f32, h: f32, tint: [f32; 4]) -> DrawCmd {
    DrawCmd { tex: 0, dst: [x, y, w, h], src: [0.0, 0.0, 1.0, 1.0], tint, rot: 0.0 }
}

/// A section heading near the top of a selection screen.
fn draw_heading(cmds: &mut Vec<DrawCmd>, label: &str) {
    draw_text_centered(cmds, SCREEN_W / 2.0, 100.0, label, 4.0, [1.0, 0.95, 0.7, 1.0]);
}

/// A vertical list of entries centred on-screen, the cursor row highlighted.
/// Disabled entries (`enabled == false`) are dimmed.
fn draw_menu_list(cmds: &mut Vec<DrawCmd>, top: f32, entries: &[(&str, bool)], cursor: usize) {
    let px = 4.0;
    let row_h = 42.0;
    for (i, (label, enabled)) in entries.iter().enumerate() {
        let y = top + i as f32 * row_h;
        let selected = i == cursor;
        let color = if !enabled {
            [0.4, 0.4, 0.45, 1.0]
        } else if selected {
            [1.0, 1.0, 0.5, 1.0]
        } else {
            [0.85, 0.85, 0.9, 1.0]
        };
        if selected {
            let w = text_width(label, px) + 40.0;
            cmds.push(fill(SCREEN_W / 2.0 - w / 2.0, y - 8.0, w, 36.0, [0.25, 0.20, 0.10, 0.85]));
            draw_text(cmds, SCREEN_W / 2.0 - w / 2.0 + 6.0, y, ">", px, [1.0, 1.0, 0.5, 1.0]);
        }
        draw_text_centered(cmds, SCREEN_W / 2.0, y, label, px, color);
    }
}

/// One OPTION row: a left-aligned label and (if any) a `< value >` on the
/// right; the focused row is highlighted with a bar + arrows.
fn draw_option_row(cmds: &mut Vec<DrawCmd>, y: f32, label: &str, value: &str, selected: bool) {
    let px = 4.0;
    let (lx, rx) = (130.0, 520.0);
    let color = if selected { [1.0, 1.0, 0.5, 1.0] } else { [0.85, 0.85, 0.9, 1.0] };
    if selected {
        cmds.push(fill(lx - 14.0, y - 8.0, rx - lx + 60.0, 36.0, [0.25, 0.20, 0.10, 0.85]));
    }
    draw_text(cmds, lx, y, label, px, color);
    if !value.is_empty() {
        if selected {
            draw_text(cmds, rx - 36.0, y, "<", px, color);
            draw_text(cmds, rx + 36.0, y, ">", px, color);
        }
        draw_text_centered(cmds, rx, y, value, px, color);
    }
}

/// Wrap a [`MenuApp`] into the engine's per-frame update closure.
pub fn make_menu_update(mut app: MenuApp) -> impl FnMut(&Input) -> Frame + 'static {
    move |inp| app.update(inp)
}
