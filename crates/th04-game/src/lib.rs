//! TH04 game: builds GPU textures from the decoded assets and draws the live
//! `StageSim` through `th06-engine`. Used by the native binary (offscreen
//! screenshots + a windowed `play` loop) and the WASM build (`web::start_game`).

use std::collections::HashMap;

use th04_formats::bft::Bft;
use th04_formats::boss::BossKind;
use th04_formats::effects::EffectKind;
use th04_formats::map::{self, Map};
use th04_formats::mpn::Mpn;
use th04_formats::par::Archive;
use th04_formats::player::Input;
use th04_formats::sim::StageSim;
use th04_formats::stage::Std;
use th06_engine::{DrawCmd, Engine, Frame, Key, Texture};

pub mod font;
pub mod menu;

#[cfg(target_arch = "wasm32")]
pub mod web;

// Where the 384×368 playfield sits within the 640×480 frame.
const PF_LEFT: f32 = 128.0;
const PF_TOP: f32 = 40.0;
const PF_W: f32 = 384.0;
const PF_H: f32 = 368.0;

/// One player character's sprite: base texture index (neutral cel; banking
/// cels follow it), how many cels, the cel size and whether a real sprite
/// loaded (vs. a solid-colour fallback).
#[derive(Clone, Copy)]
pub struct PlayerSprite {
    base: usize,
    cels: usize,
    wh: (f32, f32),
    has: bool,
}

/// Texture indices + metadata the per-frame draw needs (the textures
/// themselves are owned by the engine once the game loop starts).
pub struct DrawData {
    /// Player sprites by character: index 0 = Reimu (`MIKO.BFT`), 1 = Marisa
    /// (`MARI.BFT`); chosen at draw time from the player's `shot_type`.
    players: [PlayerSprite; 2],
    /// Background tiles are packed into one atlas texture so the whole
    /// background draws in a single batch (per-tile textures = hundreds of
    /// draw calls, which made WebGL drop tiles / flicker).
    tile_atlas: usize,
    tile_cols: usize,
    atlas_w: f32,
    atlas_h: f32,
    ntiles: usize,
    cel_idx: HashMap<u16, (usize, f32, f32)>,
    map: Option<Map>,
    section_order: Vec<u8>,
}

/// 0-based stage index from an `STnn.STD` filename (`ST00.STD` → 0). Defaults
/// to 0 if the digits can't be parsed.
pub fn stage_index(std_name: &str) -> usize {
    std_name
        .trim_start_matches(|c: char| !c.is_ascii_digit())
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .unwrap_or(0)
}

/// Load a player character's sprite (cels 0 = neutral, 1 = lean left, 2 = lean
/// right) into `textures`, or push a single solid fallback if it's missing.
fn load_player_sprite(engine: &Engine, arc: &Archive, name: &str, fallback: [u8; 4], textures: &mut Vec<Texture>) -> PlayerSprite {
    let base = textures.len();
    match arc.get(name).and_then(|b| Bft::parse(&b)) {
        Some(b) => {
            let cels = b.count.min(3).max(1);
            for n in 0..cels {
                textures.push(engine.create_texture(&b.decode_rgba(n, Some(0)).unwrap(), b.width as u32, b.height as u32));
            }
            PlayerSprite { base, cels, wh: (b.width as f32, b.height as f32), has: true }
        }
        None => {
            textures.push(engine.create_texture(&fallback, 1, 1));
            PlayerSprite { base, cels: 1, wh: (16.0, 20.0), has: false }
        }
    }
}

/// Build all stage textures + draw metadata + the parsed `Std`, without
/// creating the sim (so the menu can defer that until a character is chosen).
pub fn build_stage(engine: &Engine, arc: &Archive, std_name: &str) -> (Vec<Texture>, DrawData, Std) {
    let white = engine.create_texture(&[255, 255, 255, 255], 1, 1);
    let mut textures: Vec<Texture> = vec![white];

    // Both player characters, so the menu's character choice draws correctly.
    let players = [
        load_player_sprite(engine, arc, "MIKO.BFT", [255, 120, 120, 255], &mut textures), // Reimu
        load_player_sprite(engine, arc, "MARI.BFT", [102, 179, 255, 255], &mut textures), // Marisa
    ];

    // Background tileset (MPN) + layout (MAP), packed into one atlas texture.
    let mpn = arc.get(&std_name.replace(".STD", ".MPN")).and_then(|b| Mpn::parse(&b));
    let map = arc.get(&std_name.replace(".STD", ".MAP")).and_then(|b| Map::parse(&b));
    let tile_cols = 16usize;
    let (tile_atlas, atlas_w, atlas_h, ntiles) = match &mpn {
        Some(m) if m.count > 0 => {
            let rows = m.count.div_ceil(tile_cols);
            let (aw, ah) = (tile_cols * 16, rows * 16);
            let mut atlas = vec![0u8; aw * ah * 4];
            for ti in 0..m.count {
                let tile = m.decode_tile(ti, None).unwrap(); // 16x16 RGBA
                let (ax, ay) = ((ti % tile_cols) * 16, (ti / tile_cols) * 16);
                for y in 0..16 {
                    let s = (y * 16) * 4;
                    let d = ((ay + y) * aw + ax) * 4;
                    atlas[d..d + 64].copy_from_slice(&tile[s..s + 64]);
                }
            }
            let idx = textures.len();
            textures.push(engine.create_texture(&atlas, aw as u32, ah as u32));
            (idx, aw as f32, ah as f32, m.count)
        }
        _ => (0, 1.0, 1.0, 0),
    };

    // Sprite cels at their PAT_* bases (main_pat.h).
    let mut cel_idx = HashMap::new();
    let sheets = [
        ("MIKOD.BFT".to_string(), 3u16),
        ("MIKO32.BFT".to_string(), 4),
        ("MIKO16.BFT".to_string(), 38),
        (std_name.replace(".STD", ".BFT"), 128),
    ];
    for (name, base) in &sheets {
        if let Some(b) = arc.get(name).and_then(|d| Bft::parse(&d)) {
            for n in 0..b.count {
                if let Some(rgba) = b.decode_rgba(n, Some(0)) {
                    let idx = textures.len();
                    textures.push(engine.create_texture(&rgba, b.width as u32, b.height as u32));
                    cel_idx.insert(base + n as u16, (idx, b.width as f32, b.height as f32));
                }
            }
        }
    }

    let std = Std::parse(&arc.get(std_name).unwrap_or_default()).expect("parse STD");
    let section_order = std.map_section_order.clone();
    let dd = DrawData {
        players,
        tile_atlas,
        tile_cols,
        atlas_w,
        atlas_h,
        ntiles,
        cel_idx,
        map,
        section_order,
    };
    (textures, dd, std)
}

/// 0-based player-character index from a `shot_type` (0/1 = Reimu, 2/3 = Marisa).
fn player_char(shot_type: u8) -> usize {
    (shot_type >= 2) as usize
}

/// The end-of-stage boss for a stage + character (`STnn.STD` → stage index nn).
pub fn boss_for(std_name: &str, shot_type: u8) -> Option<BossKind> {
    BossKind::for_stage(stage_index(std_name), shot_type >= 2)
}

/// Build all textures + draw metadata + the stage sim from an archive.
pub fn setup(engine: &Engine, arc: &Archive, std_name: &str, shot_type: u8) -> (Vec<Texture>, DrawData, StageSim) {
    let (textures, dd, std) = build_stage(engine, arc, std_name);
    let sim = StageSim::new(std, shot_type, boss_for(std_name, shot_type));
    (textures, dd, sim)
}

/// Build the texture set + a [`menu::MenuApp`] for the title→menu→game flow.
/// `title_img` is the decoded title art (RGBA, width, height) — typically
/// `OP1.PI` from the `幻想郷ED.DAT` archive; pass `None` for a text-only title.
pub fn setup_menu(
    engine: &Engine,
    arc: &Archive,
    std_name: &str,
    title_img: Option<(Vec<u8>, u32, u32)>,
) -> (Vec<Texture>, menu::MenuApp) {
    let (mut textures, dd, std) = build_stage(engine, arc, std_name);
    let title_tex = title_img.map(|(rgba, w, h)| {
        let idx = textures.len();
        textures.push(engine.create_texture(&rgba, w, h));
        (idx, w as f32, h as f32)
    });
    let app = menu::MenuApp::new(dd, std, std_name.to_string(), title_tex);
    (textures, app)
}

/// 3×5 bitmap digits (one byte per row, low 3 bits, MSB = left column).
const DIGITS: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111], // 0
    [0b010, 0b110, 0b010, 0b010, 0b111], // 1
    [0b111, 0b001, 0b111, 0b100, 0b111], // 2
    [0b111, 0b001, 0b111, 0b001, 0b111], // 3
    [0b101, 0b101, 0b111, 0b001, 0b001], // 4
    [0b111, 0b100, 0b111, 0b001, 0b111], // 5
    [0b111, 0b100, 0b111, 0b101, 0b111], // 6
    [0b111, 0b001, 0b001, 0b001, 0b001], // 7
    [0b111, 0b101, 0b111, 0b101, 0b111], // 8
    [0b111, 0b101, 0b111, 0b001, 0b111], // 9
];

/// Absolute-position rectangle (screen space), tex 0 tinted.
fn rect(x: f32, y: f32, w: f32, h: f32, tint: [f32; 4]) -> DrawCmd {
    DrawCmd { tex: 0, dst: [x, y, w, h], src: [0.0, 0.0, 1.0, 1.0], tint, rot: 0.0 }
}

/// Draw `value` right-aligned ending at `x_right` using the built-in 3×5 font.
fn push_number(cmds: &mut Vec<DrawCmd>, x_right: f32, y: f32, value: i64, px: f32, color: [f32; 4]) {
    let s = value.max(0).to_string();
    let advance = px * 4.0; // 3 cols + 1 gap
    let mut x = x_right - s.len() as f32 * advance;
    for ch in s.bytes() {
        let g = &DIGITS[(ch - b'0') as usize];
        for (row, bits) in g.iter().enumerate() {
            for col in 0..3 {
                if bits & (0b100 >> col) != 0 {
                    cmds.push(rect(x + col as f32 * px, y + row as f32 * px, px, px, color));
                }
            }
        }
        x += advance;
    }
}

fn solid(x: f32, y: f32, w: f32, h: f32, tint: [f32; 4]) -> DrawCmd {
    DrawCmd { tex: 0, dst: [PF_LEFT + x - w / 2.0, PF_TOP + y - h / 2.0, w, h], src: [0.0, 0.0, 1.0, 1.0], tint, rot: 0.0 }
}
fn sprite(tex: usize, x: f32, y: f32, w: f32, h: f32) -> DrawCmd {
    DrawCmd { tex, dst: [PF_LEFT + x - w / 2.0, PF_TOP + y - h / 2.0, w, h], src: [0.0, 0.0, 1.0, 1.0], tint: [1.0; 4], rot: 0.0 }
}

/// Build the draw-command list for the current sim state.
pub fn draw_frame(sim: &StageSim, dd: &DrawData) -> Vec<DrawCmd> {
    let mut cmds = Vec::new();
    // Playfield backdrop.
    cmds.push(DrawCmd { tex: 0, dst: [PF_LEFT, PF_TOP, PF_W, PF_H], src: [0.0, 0.0, 1.0, 1.0], tint: [0.04, 0.04, 0.10, 1.0], rot: 0.0 });
    // Scrolling tile background. The world scrolls *downward* (the player flies
    // forward; scenery flows toward them at the bottom). map_section_order[0]
    // is the start, shown at the bottom; later sections enter from the top.
    if let Some(mp) = &dd.map {
        let total_rows = (dd.section_order.len() * map::ROWS_PER_SECTION) as i32;
        let screen_rows = PF_H as i32 / 16; // 23 visible tile rows
        let bottom_row = sim.frame as i32 / 16; // progress, increases over time
        let frac = (sim.frame % 16) as f32; // sub-tile downward offset
        for sr in -1..=(screen_rows + 1) {
            // Screen row 0 = top = newest; bottom = oldest. Higher rows scroll
            // in from the top as `bottom_row` grows, so content moves down.
            let bg_row = bottom_row + (screen_rows - sr);
            if bg_row < 0 || bg_row >= total_rows {
                continue;
            }
            let sec = dd.section_order[bg_row as usize / map::ROWS_PER_SECTION] as usize;
            let rs = bg_row as usize % map::ROWS_PER_SECTION;
            if sec >= mp.sections.len() {
                continue;
            }
            let sy = PF_TOP + (sr * 16) as f32 + frac;
            for col in 0..map::TILES_X {
                let ti = Map::tile_index(mp.sections[sec][rs][col]);
                if ti < dd.ntiles {
                    // UV into the tile atlas (all tiles share one texture).
                    let (ax, ay) = ((ti % dd.tile_cols) * 16, (ti / dd.tile_cols) * 16);
                    let (u0, v0) = (ax as f32 / dd.atlas_w, ay as f32 / dd.atlas_h);
                    let (u1, v1) = ((ax + 16) as f32 / dd.atlas_w, (ay + 16) as f32 / dd.atlas_h);
                    cmds.push(DrawCmd {
                        tex: dd.tile_atlas,
                        dst: [PF_LEFT + (col * 16) as f32, sy, 16.0, 16.0],
                        src: [u0, v0, u1, v1],
                        tint: [1.0; 4],
                        rot: 0.0,
                    });
                }
            }
        }
    }
    // Enemies.
    for e in &sim.enemies {
        let (px, py) = (e.x as f32 / 16.0, e.y as f32 / 16.0);
        match dd.cel_idx.get(&e.anim_patnum()) {
            Some(&(idx, w, h)) => cmds.push(sprite(idx, px, py, w, h)),
            None => cmds.push(solid(px, py, 18.0, 18.0, [1.0, 0.3, 0.3, 1.0])),
        }
    }
    // Boss telegraphs (gather/circle/spark): non-damaging charge-up cues,
    // faded by age so they pulse.
    for fx in &sim.effects.effects {
        let life = 1.0 - (fx.age as f32 / fx.ttl.max(1) as f32);
        let (sz, c) = match fx.kind {
            EffectKind::Gather => (8.0 + 28.0 * (1.0 - life), [0.6, 0.9, 1.0, 0.30 * life]),
            EffectKind::CircleShrink => (8.0 + 24.0 * life, [1.0, 1.0, 1.0, 0.30 * life]),
            EffectKind::CircleGrow => (8.0 + 24.0 * (1.0 - life), [1.0, 0.9, 0.5, 0.30 * life]),
            EffectKind::Spark => (10.0 + 30.0 * (1.0 - life), [1.0, 0.7, 0.2, 0.45 * life]),
        };
        cmds.push(solid(fx.x as f32 / 16.0, fx.y as f32 / 16.0, sz, sz, c));
    }
    if let Some(b) = &sim.boss {
        if !b.defeated {
            // Spawn-rays (Kurumi): dotted line from the boss to the growing tip.
            for r in b.rays.iter().filter(|r| r.flag != 0) {
                for k in 0..=6 {
                    let t = k as f32 / 6.0;
                    let x = (r.ox as f32 + (r.tx - r.ox) as f32 * t) / 16.0;
                    let y = (r.oy as f32 + (r.ty - r.oy) as f32 * t) / 16.0;
                    cmds.push(solid(x, y, 6.0, 6.0, [0.6, 0.8, 1.0, 0.9]));
                }
            }
            // Orbiting satellites (Reimu orbs / Marisa bits).
            for o in b.orbits.iter().filter(|o| o.flag != 0) {
                cmds.push(solid(o.cx as f32 / 16.0, o.cy as f32 / 16.0, 16.0, 16.0, [0.7, 0.85, 1.0, 1.0]));
            }
            cmds.push(solid(b.x as f32 / 16.0, b.y as f32 / 16.0, 56.0, 56.0, [0.85, 0.3, 0.95, 1.0]));
        }
    }
    if let Some(m) = &sim.midboss {
        if !m.defeated {
            cmds.push(solid(m.x as f32 / 16.0, m.y as f32 / 16.0, 40.0, 40.0, [0.3, 0.9, 0.9, 1.0]));
        }
    }
    for b in &sim.bullets.bullets {
        if b.active {
            cmds.push(solid(b.x as f32 / 16.0, b.y as f32 / 16.0, 7.0, 7.0, [1.0, 1.0, 0.6, 1.0]));
        }
    }
    for s in &sim.player.shots {
        if s.active {
            cmds.push(solid(s.x as f32 / 16.0, s.y as f32 / 16.0, 4.0, 10.0, [0.5, 1.0, 1.0, 1.0]));
        }
    }
    // Dropped items (placeholder colours by kind until the item sprites are mapped).
    for it in &sim.items {
        let c = match it.kind {
            0 => [1.0, 0.25, 0.25, 1.0], // power (red)
            1 => [0.3, 0.55, 1.0, 1.0],  // point (blue)
            _ => [0.95, 0.9, 0.35, 1.0], // other (yellow)
        };
        cmds.push(solid(it.x as f32 / 16.0, it.y as f32 / 16.0, 10.0, 10.0, c));
    }

    let (ppx, ppy) = (sim.player.x as f32 / 16.0, sim.player.y as f32 / 16.0);
    // Blink while invulnerable.
    let show = sim.player.invuln == 0 || (sim.frame / 4) % 2 == 0;
    if show {
        let ps = &dd.players[player_char(sim.player.shot_type)];
        if ps.has {
            // Banking cel from the player's lean (clamped to what's available).
            let cel = match sim.player.facing.signum() {
                -1 => 1,
                1 => 2,
                _ => 0,
            }
            .min(ps.cels.saturating_sub(1));
            cmds.push(sprite(ps.base + cel, ppx, ppy, ps.wh.0, ps.wh.1));
        } else {
            cmds.push(solid(ppx, ppy, 16.0, 20.0, [0.4, 0.7, 1.0, 1.0]));
        }
    }

    // Letterbox: hide anything drawn outside the playfield (the scrolling tiles
    // overshoot its edges; the original masks them under the HUD/border).
    let black = [0.0, 0.0, 0.0, 1.0];
    let mask = |x: f32, y: f32, w: f32, h: f32| DrawCmd { tex: 0, dst: [x, y, w, h], src: [0.0, 0.0, 1.0, 1.0], tint: black, rot: 0.0 };
    cmds.push(mask(0.0, 0.0, 640.0, PF_TOP));
    cmds.push(mask(0.0, PF_TOP + PF_H, 640.0, 480.0 - (PF_TOP + PF_H)));
    cmds.push(mask(0.0, PF_TOP, PF_LEFT, PF_H));
    cmds.push(mask(PF_LEFT + PF_W, PF_TOP, 640.0 - (PF_LEFT + PF_W), PF_H));

    // HUD in the right panel (graphical; the bitmap digits stand in for the
    // original gaiji font). Panel inner-x ≈ 520..632.
    let px = PF_LEFT + PF_W + 8.0; // 520
    let white = [1.0, 1.0, 1.0, 1.0];
    // Score (right-aligned).
    push_number(&mut cmds, 632.0, 52.0, sim.score, 3.0, white);
    // Lives (green) and bombs (blue) as icon rows.
    for i in 0..sim.player.lives.max(0).min(8) {
        cmds.push(rect(px + i as f32 * 12.0, 92.0, 9.0, 9.0, [0.4, 1.0, 0.5, 1.0]));
    }
    for i in 0..sim.player.bombs.max(0).min(8) {
        cmds.push(rect(px + i as f32 * 12.0, 116.0, 9.0, 9.0, [0.5, 0.7, 1.0, 1.0]));
    }
    // Power bar (0..128).
    let pw = 104.0;
    cmds.push(rect(px, 148.0, pw, 8.0, [0.2, 0.2, 0.25, 1.0]));
    let fill = pw * (sim.player.power as f32 / 128.0).min(1.0);
    cmds.push(rect(px, 148.0, fill, 8.0, [1.0, 0.85, 0.3, 1.0]));

    // Boss / midboss HP bar across the top of the playfield.
    let hp = match (&sim.boss, &sim.midboss) {
        (Some(b), _) if !b.defeated => Some((b.hp, b.max_hp)),
        (_, Some(m)) if !m.defeated => Some((m.hp, 620)),
        _ => None,
    };
    if let Some((cur, max)) = hp {
        cmds.push(rect(PF_LEFT, PF_TOP + 2.0, PF_W, 5.0, [0.2, 0.05, 0.1, 1.0]));
        let w = PF_W * (cur.max(0) as f32 / max as f32);
        cmds.push(rect(PF_LEFT, PF_TOP + 2.0, w, 5.0, [1.0, 0.3, 0.4, 1.0]));
    }
    cmds
}

/// Map engine input to the player's input.
pub fn map_input(inp: &th06_engine::Input) -> Input {
    Input {
        left: inp.held(Key::Left),
        right: inp.held(Key::Right),
        up: inp.held(Key::Up),
        down: inp.held(Key::Down),
        shoot: inp.held(Key::Shoot),
        focus: inp.held(Key::Focus),
        bomb: inp.pressed(Key::Bomb),
    }
}

/// The interactive update closure: step the sim from input, draw the frame.
pub fn make_update(mut sim: StageSim, dd: DrawData) -> impl FnMut(&th06_engine::Input) -> Frame {
    move |inp| {
        sim.step(&map_input(inp));
        Frame { cmds: draw_frame(&sim, &dd), bg: None, quit: false }
    }
}
