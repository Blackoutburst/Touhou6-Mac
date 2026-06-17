//! TH04 game harness. Two modes:
//!   th04-game title <archive> [out.png]            — render the title screen
//!   th04-game stage <archive> <STnn.STD> [frame] [out.png]
//!       — run the headless stage sim N frames, then draw that frame
//!         (background + enemies + bullets + player + shots) through th06-engine.
//!
//! Entities are drawn as tinted markers for now (placeholder for real BFNT
//! sprites); this proves the StageSim → engine render path. Verified offscreen
//! via render_to_image.

use th04_formats::bft::Bft;
use th04_formats::map::{self, Map};
use th04_formats::mpn::Mpn;
use th04_formats::par::Archive;
use th04_formats::pi::Pi;
use th04_formats::boss::{Boss, Midboss};
use th04_formats::player::Input;
use th04_formats::sim::{Phase, StageSim};
use th04_formats::stage::Std;
use th06_engine::{DrawCmd, Engine, SCREEN_H, SCREEN_W};

// Where the 384×368 playfield sits within the 640×480 frame.
const PF_LEFT: f32 = 128.0;
const PF_TOP: f32 = 40.0;
const PF_W: f32 = 384.0;
const PF_H: f32 = 368.0;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("stage") => stage(&args[1..]),
        _ => title(&args[1..]),
    }
}

fn title(a: &[String]) {
    let archive = a.first().expect("usage: th04-game title <archive> [out.png]");
    let out = a.get(1).cloned().unwrap_or_else(|| "title.png".into());
    let arc = Archive::parse(std::fs::read(archive).expect("read archive")).expect("parse");
    let pi = Pi::parse(&arc.get("OP1.PI").expect("OP1.PI")).expect("parse PI");
    let engine = Engine::new();
    let tex = engine.create_texture(&pi.to_rgba(), pi.width as u32, pi.height as u32);
    let x = (SCREEN_W as f32 - pi.width as f32) / 2.0;
    let y = (SCREEN_H as f32 - pi.height as f32) / 2.0;
    let cmd = DrawCmd { tex: 0, dst: [x, y, pi.width as f32, pi.height as f32], src: [0.0, 0.0, 1.0, 1.0], tint: [1.0; 4], rot: 0.0 };
    let frame = engine.render_to_image(&[cmd], &[&tex], None);
    image::save_buffer(&out, &frame, SCREEN_W, SCREEN_H, image::ColorType::Rgba8).expect("save");
    println!("wrote {}", out);
}

fn stage(a: &[String]) {
    if a.len() < 2 {
        eprintln!("usage: th04-game stage <archive> <STnn.STD> [frame] [out.png]");
        std::process::exit(2);
    }
    let arc = Archive::parse(std::fs::read(&a[0]).expect("read archive")).expect("parse archive");
    let std_name = &a[1];
    let until: u32 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(300);
    let out = a.get(3).cloned().unwrap_or_else(|| "stage.png".into());

    let force_boss = a.get(2).map(|s| s == "boss").unwrap_or(false);
    let force_midboss = a.get(2).map(|s| s == "midboss").unwrap_or(false);

    let std = Std::parse(&arc.get(std_name).unwrap()).expect("parse STD");
    let section_order = std.map_section_order.clone();
    let mut sim = StageSim::new(std, 0);
    // Run the sim up to the requested frame, holding shoot + weaving.
    for f in 0..until {
        let mut input = Input::default();
        input.shoot = true;
        input.left = (f / 64) % 2 == 0;
        input.right = !input.left;
        sim.step(&input);
    }
    // Demo: drop straight into the boss / midboss fight for a screenshot.
    if force_boss || force_midboss {
        sim.player.gameover = false;
        sim.player.lives = 2;
        if force_boss {
            sim.phase = Phase::Boss;
            if sim.boss.is_none() {
                sim.boss = Some(Boss::new(1500, 4));
            }
        } else if sim.midboss.is_none() {
            sim.midboss = Some(Midboss::new(192 * 16));
        }
        for _ in 0..160 {
            let mut input = Input::default();
            input.shoot = true;
            sim.step(&input);
        }
    }

    let engine = Engine::new();

    // Texture 0 = 1×1 white (markers/backdrop), 1 = player; then background
    // tiles, then sprite cels (indexed via `cel_idx`).
    let white = engine.create_texture(&[255, 255, 255, 255], 1, 1);

    let mari = arc.get("MARI.BFT").and_then(|b| Bft::parse(&b));
    let (player_tex, player_wh) = match &mari {
        Some(b) => (engine.create_texture(&b.decode_rgba(0, Some(0)).unwrap(), b.width as u32, b.height as u32), (b.width as f32, b.height as f32)),
        None => (engine.create_texture(&[102, 179, 255, 255], 1, 1), (16.0, 20.0)),
    };
    let mut textures: Vec<th06_engine::Texture> = vec![white, player_tex];
    const PLAYER: usize = 1;

    // Background tileset (MPN — the real stage palette) + layout (MAP).
    let mpn = arc.get(&std_name.replace(".STD", ".MPN")).and_then(|b| Mpn::parse(&b));
    let map = arc.get(&std_name.replace(".STD", ".MAP")).and_then(|b| Map::parse(&b));
    let tile_base = textures.len();
    let ntiles = match &mpn {
        Some(m) => {
            for i in 0..m.count {
                let rgba = m.decode_tile(i, None).unwrap();
                textures.push(engine.create_texture(&rgba, 16, 16));
            }
            m.count
        }
        None => 0,
    };

    // Sprite cels at their PAT_* bases (main_pat.h): shared sheets, then the
    // stage sheet at PAT_STAGE = 128.
    let mut cel_idx: std::collections::HashMap<u16, (usize, f32, f32)> = std::collections::HashMap::new();
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

    let mut cmds: Vec<DrawCmd> = Vec::new();
    let solid = |x: f32, y: f32, w: f32, h: f32, tint: [f32; 4]| DrawCmd {
        tex: 0,
        dst: [PF_LEFT + x - w / 2.0, PF_TOP + y - h / 2.0, w, h],
        src: [0.0, 0.0, 1.0, 1.0],
        tint,
        rot: 0.0,
    };
    let sprite = |tex: usize, x: f32, y: f32, w: f32, h: f32| DrawCmd {
        tex,
        dst: [PF_LEFT + x - w / 2.0, PF_TOP + y - h / 2.0, w, h],
        src: [0.0, 0.0, 1.0, 1.0],
        tint: [1.0; 4],
        rot: 0.0,
    };

    // Dark playfield backdrop.
    cmds.push(DrawCmd { tex: 0, dst: [PF_LEFT, PF_TOP, PF_W, PF_H], src: [0.0, 0.0, 1.0, 1.0], tint: [0.04, 0.04, 0.10, 1.0], rot: 0.0 });
    // Scrolling tile background: walk map_section_order, 5 rows per section.
    if let (Some(_), Some(mp)) = (&mpn, &map) {
        let total_rows = (section_order.len() * map::ROWS_PER_SECTION) as i32;
        let scroll_px = until as i32; // ~1px/frame
        let top_row = scroll_px / 16;
        let frac = (scroll_px % 16) as f32;
        let rows_on_screen = (PF_H as i32 / 16) + 1;
        for sr in 0..=rows_on_screen {
            let bg_row = top_row + sr;
            if bg_row < 0 || bg_row >= total_rows {
                continue;
            }
            let sec = section_order[bg_row as usize / map::ROWS_PER_SECTION] as usize;
            let rs = bg_row as usize % map::ROWS_PER_SECTION;
            if sec >= mp.sections.len() {
                continue;
            }
            let sy = PF_TOP + (sr * 16) as f32 - frac;
            for col in 0..map::TILES_X {
                let ti = Map::tile_index(mp.sections[sec][rs][col]);
                if ti < ntiles {
                    cmds.push(DrawCmd {
                        tex: tile_base + ti,
                        dst: [PF_LEFT + (col * 16) as f32, sy, 16.0, 16.0],
                        src: [0.0, 0.0, 1.0, 1.0],
                        tint: [1.0; 4],
                        rot: 0.0,
                    });
                }
            }
        }
    }
    // Enemies as real BFNT sprites, animated (patnum_base + anim cel → global cel).
    for e in &sim.enemies {
        let (px, py) = (e.x as f32 / 16.0, e.y as f32 / 16.0);
        match cel_idx.get(&e.anim_patnum()) {
            Some(&(idx, w, h)) => cmds.push(sprite(idx, px, py, w, h)),
            None => cmds.push(solid(px, py, 18.0, 18.0, [1.0, 0.3, 0.3, 1.0])),
        }
    }
    // Boss / midboss (placeholder markers until the CD2 sprites are mapped).
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
    // Bullets + player shots as markers (their sprite sheets aren't decoded yet).
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
    // Player sprite.
    let (ppx, ppy) = (sim.player.x as f32 / 16.0, sim.player.y as f32 / 16.0);
    if mari.is_some() {
        cmds.push(sprite(PLAYER, ppx, ppy, player_wh.0, player_wh.1));
    } else {
        cmds.push(solid(ppx, ppy, 16.0, 20.0, [0.4, 0.7, 1.0, 1.0]));
    }

    let texes: Vec<&th06_engine::Texture> = textures.iter().collect();
    let frame_img = engine.render_to_image(&cmds, &texes, None);
    image::save_buffer(&out, &frame_img, SCREEN_W, SCREEN_H, image::ColorType::Rgba8).expect("save png");
    println!(
        "{:?} frame {}: {} enemies, {} bullets, {} shots, boss_hp {:?} -> {}",
        sim.phase, until, sim.alive_enemies(), sim.bullets.active_count(), sim.player.active_shots(),
        sim.boss.as_ref().map(|b| b.hp), out
    );
}
