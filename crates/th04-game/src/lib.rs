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
const PLAYER: usize = 1;

/// Texture indices + metadata the per-frame draw needs (the textures
/// themselves are owned by the engine once the game loop starts).
pub struct DrawData {
    player_wh: (f32, f32),
    has_player_sprite: bool,
    tile_base: usize,
    ntiles: usize,
    cel_idx: HashMap<u16, (usize, f32, f32)>,
    map: Option<Map>,
    section_order: Vec<u8>,
}

/// Build all textures + draw metadata + the stage sim from an archive.
pub fn setup(engine: &Engine, arc: &Archive, std_name: &str, shot_type: u8) -> (Vec<Texture>, DrawData, StageSim) {
    let white = engine.create_texture(&[255, 255, 255, 255], 1, 1);

    let mari = arc.get("MARI.BFT").and_then(|b| Bft::parse(&b));
    let (player_tex, player_wh) = match &mari {
        Some(b) => (
            engine.create_texture(&b.decode_rgba(0, Some(0)).unwrap(), b.width as u32, b.height as u32),
            (b.width as f32, b.height as f32),
        ),
        None => (engine.create_texture(&[102, 179, 255, 255], 1, 1), (16.0, 20.0)),
    };
    let mut textures: Vec<Texture> = vec![white, player_tex];

    // Background tileset (MPN) + layout (MAP).
    let mpn = arc.get(&std_name.replace(".STD", ".MPN")).and_then(|b| Mpn::parse(&b));
    let map = arc.get(&std_name.replace(".STD", ".MAP")).and_then(|b| Map::parse(&b));
    let tile_base = textures.len();
    let ntiles = match &mpn {
        Some(m) => {
            for i in 0..m.count {
                textures.push(engine.create_texture(&m.decode_tile(i, None).unwrap(), 16, 16));
            }
            m.count
        }
        None => 0,
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
        tile_base,
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
    // Scrolling tile background.
    if let Some(mp) = &dd.map {
        let total_rows = (dd.section_order.len() * map::ROWS_PER_SECTION) as i32;
        let scroll_px = sim.frame as i32;
        let top_row = scroll_px / 16;
        let frac = (scroll_px % 16) as f32;
        for sr in 0..=(PF_H as i32 / 16 + 1) {
            let bg_row = top_row + sr;
            if bg_row < 0 || bg_row >= total_rows {
                continue;
            }
            let sec = dd.section_order[bg_row as usize / map::ROWS_PER_SECTION] as usize;
            let rs = bg_row as usize % map::ROWS_PER_SECTION;
            if sec >= mp.sections.len() {
                continue;
            }
            let sy = PF_TOP + (sr * 16) as f32 - frac;
            for col in 0..map::TILES_X {
                let ti = Map::tile_index(mp.sections[sec][rs][col]);
                if ti < dd.ntiles {
                    cmds.push(DrawCmd { tex: dd.tile_base + ti, dst: [PF_LEFT + (col * 16) as f32, sy, 16.0, 16.0], src: [0.0, 0.0, 1.0, 1.0], tint: [1.0; 4], rot: 0.0 });
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
    let (ppx, ppy) = (sim.player.x as f32 / 16.0, sim.player.y as f32 / 16.0);
    // Blink while invulnerable.
    let show = sim.player.invuln == 0 || (sim.frame / 4) % 2 == 0;
    if show {
        if dd.has_player_sprite {
            cmds.push(sprite(PLAYER, ppx, ppy, dd.player_wh.0, dd.player_wh.1));
        } else {
            cmds.push(solid(ppx, ppy, 16.0, 20.0, [0.4, 0.7, 1.0, 1.0]));
        }
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
