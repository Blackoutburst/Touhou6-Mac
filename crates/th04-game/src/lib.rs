//! TH04 game: builds GPU textures from the decoded assets and draws the live
//! `StageSim` through `th06-engine`. Used by the native binary (offscreen
//! screenshots + a windowed `play` loop) and the WASM build (`web::start_game`).

use std::collections::HashMap;

use th04_formats::bft::Bft;
use th04_formats::map::{self, Map};
use th04_formats::mpn::Mpn;
use th04_formats::par::Archive;
use th04_formats::player::Input;
use th04_formats::sim::StageSim;
use th04_formats::stage::Std;
use th06_engine::{DrawCmd, Engine, Frame, Key, Texture};

#[cfg(target_arch = "wasm32")]
pub mod web;

// Where the 384×368 playfield sits within the 640×480 frame.
const PF_LEFT: f32 = 128.0;
const PF_TOP: f32 = 40.0;
const PF_W: f32 = 384.0;
const PF_H: f32 = 368.0;

/// Texture indices + metadata the per-frame draw needs (the textures
/// themselves are owned by the engine once the game loop starts).
pub struct DrawData {
    player_wh: (f32, f32),
    has_player_sprite: bool,
    /// Texture index of the player's first cel; banking cels follow it.
    player_base: usize,
    player_cels: usize,
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

/// Build all textures + draw metadata + the stage sim from an archive.
pub fn setup(engine: &Engine, arc: &Archive, std_name: &str, shot_type: u8) -> (Vec<Texture>, DrawData, StageSim) {
    let white = engine.create_texture(&[255, 255, 255, 255], 1, 1);
    let mut textures: Vec<Texture> = vec![white];

    // Player cels: 0 = neutral, 1 = lean left, 2 = lean right.
    let player_base = textures.len();
    let mari = arc.get("MARI.BFT").and_then(|b| Bft::parse(&b));
    let (player_wh, player_cels) = match &mari {
        Some(b) => {
            let cels = b.count.min(3);
            for n in 0..cels {
                textures.push(engine.create_texture(&b.decode_rgba(n, Some(0)).unwrap(), b.width as u32, b.height as u32));
            }
            ((b.width as f32, b.height as f32), cels)
        }
        None => {
            textures.push(engine.create_texture(&[102, 179, 255, 255], 1, 1));
            ((16.0, 20.0), 1)
        }
    };

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
    let sim = StageSim::new(std, shot_type);
    let dd = DrawData {
        player_wh,
        has_player_sprite: mari.is_some(),
        player_base,
        player_cels,
        tile_atlas,
        tile_cols,
        atlas_w,
        atlas_h,
        ntiles,
        cel_idx,
        map,
        section_order,
    };
    (textures, dd, sim)
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
    if let Some(b) = &sim.boss {
        if !b.defeated {
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
        if dd.has_player_sprite {
            // Banking cel from the player's lean (clamped to what's available).
            let cel = match sim.player.facing.signum() {
                -1 => 1,
                1 => 2,
                _ => 0,
            }
            .min(dd.player_cels.saturating_sub(1));
            cmds.push(sprite(dd.player_base + cel, ppx, ppy, dd.player_wh.0, dd.player_wh.1));
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
