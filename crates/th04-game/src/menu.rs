//! Title screen + menu state machine, driven by the same 60 Hz `update`
//! closure that runs the stage. Flow:
//!
//! ```text
//! Title ──Z──▶ Main ──START──▶ Character ──▶ Shot ──▶ Rank ──▶ Playing
//!                │                                                  │
//!                └──QUIT──▶ exit                  clear / game over ─┘
//!                                                       │
//!                                                       ▼
//!                                                     Title
//! ```
//!
//! Textures are fixed up front (the engine owns them for the whole loop), so the
//! sim can't be built until the player has chosen a character — we keep the
//! parsed [`Std`] around and clone it into a fresh [`StageSim`] on confirm.

use th04_formats::sim::StageSim;
use th04_formats::stage::Std;
use th06_engine::{DrawCmd, Frame, Input, Key};

use crate::font::{draw_text, draw_text_centered, text_width};
use crate::{draw_frame, map_input, DrawData};

const SCREEN_W: f32 = 640.0;

/// Main-menu entries. Only START and QUIT are wired; the rest are shown (so the
/// real menu shape is visible) but disabled.
const MAIN_ENTRIES: &[(&str, bool)] = &[
    ("START", true),
    ("EXTRA START", false),
    ("PRACTICE START", false),
    ("MUSIC ROOM", false),
    ("OPTION", false),
    ("QUIT", true),
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
    Playing(Box<StageSim>),
    /// Post-stage splash; `cleared` picks the message, `t` counts it down.
    Result { cleared: bool, t: u32 },
}

pub struct MenuApp {
    screen: Screen,
    dd: DrawData,
    /// Stage-1 data, cloned into a new sim whenever a run starts.
    std: Std,
    std_name: String,
    /// Title image: texture index + draw size (None → a text-only title).
    title_tex: Option<(usize, f32, f32)>,
    // Selection carried across the character → shot → rank screens.
    character: usize,
    shot: usize,
    rank: usize,
    blink: u32,
}

impl MenuApp {
    pub fn new(dd: DrawData, std: Std, std_name: String, title_tex: Option<(usize, f32, f32)>) -> Self {
        MenuApp {
            screen: Screen::Title,
            dd,
            std,
            std_name,
            title_tex,
            character: 0,
            shot: 0,
            rank: 1, // default Normal
            blink: 0,
        }
    }

    fn shot_type(&self) -> u8 {
        (self.character as u8) * 2 + self.shot as u8
    }

    fn start_run(&mut self) {
        let shot_type = self.shot_type();
        let boss = crate::boss_for(&self.std_name, shot_type);
        let sim = StageSim::new(self.std.clone(), shot_type, boss);
        self.screen = Screen::Playing(Box::new(sim));
    }

    /// Advance one frame; returns the frame to draw (and whether to quit).
    pub fn update(&mut self, inp: &Input) -> Frame {
        self.blink = self.blink.wrapping_add(1);
        let up = inp.pressed(Key::Up) || inp.pressed(Key::Left);
        let down = inp.pressed(Key::Down) || inp.pressed(Key::Right);
        let confirm = inp.pressed(Key::Shoot) || inp.pressed(Key::Enter);
        let back = inp.pressed(Key::Pause) || inp.pressed(Key::Bomb);

        let mut quit = false;
        let mut cmds = Vec::new();

        match &mut self.screen {
            Screen::Title => {
                self.draw_backdrop(&mut cmds, false);
                if (self.blink / 30) % 2 == 0 {
                    draw_text_centered(&mut cmds, SCREEN_W / 2.0, 430.0, "PRESS Z", 4.0, [1.0; 4]);
                }
                if confirm {
                    self.screen = Screen::Main(0);
                } else if back {
                    quit = true;
                }
            }
            Screen::Main(cursor) => {
                if up && *cursor > 0 {
                    *cursor -= 1;
                }
                if down && *cursor + 1 < MAIN_ENTRIES.len() {
                    *cursor += 1;
                }
                let cur = *cursor;
                self.draw_backdrop(&mut cmds, true);
                draw_menu_list(&mut cmds, 220.0, &MAIN_ENTRIES.iter().map(|(s, e)| (*s, *e)).collect::<Vec<_>>(), cur);
                if confirm {
                    match MAIN_ENTRIES[cur] {
                        ("START", _) => self.screen = Screen::Char(self.character),
                        ("QUIT", _) => quit = true,
                        _ => {} // disabled entry: no-op
                    }
                } else if back {
                    self.screen = Screen::Title;
                }
            }
            Screen::Char(cursor) => {
                if up && *cursor > 0 {
                    *cursor -= 1;
                }
                if down && *cursor + 1 < CHARACTERS.len() {
                    *cursor += 1;
                }
                let cur = *cursor;
                self.draw_backdrop(&mut cmds, true);
                draw_title_label(&mut cmds, "SELECT CHARACTER");
                let entries: Vec<(&str, bool)> = CHARACTERS.iter().map(|s| (*s, true)).collect();
                draw_menu_list(&mut cmds, 240.0, &entries, cur);
                if confirm {
                    self.character = cur;
                    self.screen = Screen::Shot(self.shot);
                } else if back {
                    self.screen = Screen::Main(0);
                }
            }
            Screen::Shot(cursor) => {
                if up && *cursor > 0 {
                    *cursor -= 1;
                }
                if down && *cursor + 1 < SHOTS.len() {
                    *cursor += 1;
                }
                let cur = *cursor;
                self.draw_backdrop(&mut cmds, true);
                draw_title_label(&mut cmds, "SELECT SHOT TYPE");
                let entries: Vec<(&str, bool)> = SHOTS.iter().map(|s| (*s, true)).collect();
                draw_menu_list(&mut cmds, 240.0, &entries, cur);
                if confirm {
                    self.shot = cur;
                    self.screen = Screen::Rank(self.rank);
                } else if back {
                    self.screen = Screen::Char(self.character);
                }
            }
            Screen::Rank(cursor) => {
                if up && *cursor > 0 {
                    *cursor -= 1;
                }
                if down && *cursor + 1 < RANKS.len() {
                    *cursor += 1;
                }
                let cur = *cursor;
                self.draw_backdrop(&mut cmds, true);
                draw_title_label(&mut cmds, "SELECT DIFFICULTY");
                let entries: Vec<(&str, bool)> = RANKS.iter().map(|s| (*s, true)).collect();
                draw_menu_list(&mut cmds, 220.0, &entries, cur);
                draw_text_centered(&mut cmds, SCREEN_W / 2.0, 380.0, "PATTERNS ARE NORMAL RANK", 2.0, [0.6, 0.6, 0.7, 1.0]);
                if confirm {
                    self.rank = cur;
                    self.start_run();
                } else if back {
                    self.screen = Screen::Shot(self.shot);
                }
            }
            Screen::Playing(sim) => {
                if back {
                    // Abandon the run.
                    self.screen = Screen::Title;
                } else {
                    sim.step(&map_input(inp));
                    cmds = draw_frame(sim, &self.dd);
                    if sim.finished() {
                        let cleared = !sim.player.gameover;
                        self.screen = Screen::Result { cleared, t: 0 };
                    }
                }
            }
            Screen::Result { cleared, t } => {
                *t += 1;
                let cleared = *cleared;
                let done = *t > 240;
                self.draw_backdrop(&mut cmds, true);
                let (msg, col) = if cleared {
                    ("STAGE CLEAR", [0.6, 1.0, 0.7, 1.0])
                } else {
                    ("GAME OVER", [1.0, 0.5, 0.5, 1.0])
                };
                draw_text_centered(&mut cmds, SCREEN_W / 2.0, 200.0, msg, 6.0, col);
                draw_text_centered(&mut cmds, SCREEN_W / 2.0, 300.0, "PRESS Z", 3.0, [0.8; 4]);
                if confirm || back || done {
                    self.screen = Screen::Title;
                }
            }
        }

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
}

/// Solid quad on the white texture (absolute screen coords).
fn fill(x: f32, y: f32, w: f32, h: f32, tint: [f32; 4]) -> DrawCmd {
    DrawCmd { tex: 0, dst: [x, y, w, h], src: [0.0, 0.0, 1.0, 1.0], tint, rot: 0.0 }
}

/// A small section heading near the top of a selection screen.
fn draw_title_label(cmds: &mut Vec<DrawCmd>, label: &str) {
    draw_text_centered(cmds, SCREEN_W / 2.0, 110.0, label, 4.0, [1.0, 0.95, 0.7, 1.0]);
}

/// A vertical list of entries centred on-screen, the cursor row highlighted.
/// Disabled entries (`enabled == false`) are dimmed.
fn draw_menu_list(cmds: &mut Vec<DrawCmd>, top: f32, entries: &[(&str, bool)], cursor: usize) {
    let px = 4.0;
    let row_h = 44.0;
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

/// Wrap a [`MenuApp`] into the engine's per-frame update closure.
pub fn make_menu_update(mut app: MenuApp) -> impl FnMut(&Input) -> Frame + 'static {
    move |inp| app.update(inp)
}
