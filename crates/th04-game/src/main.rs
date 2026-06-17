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
use th04_formats::cdg::Cdg;
use th04_formats::par::Archive;
use th04_formats::pi::Pi;
use th04_formats::player::Input;
use th04_formats::sim::StageSim;
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

    let std = Std::parse(&arc.get(std_name).unwrap()).expect("parse STD");
    let mut sim = StageSim::new(std, 0);
    // Run the sim up to the requested frame, holding shoot + weaving.
    for f in 0..until {
        let mut input = Input::default();
        input.shoot = true;
        input.left = (f / 64) % 2 == 0;
        input.right = !input.left;
        sim.step(&input);
    }

    let engine = Engine::new();

    // Texture set: index 0 = 1x1 white (markers/backdrop), 1 = background,
    // 2 = player; the rest are sprite cels indexed by `cel_idx` (patnum → slot).
    let white = engine.create_texture(&[255, 255, 255, 255], 1, 1);

    // Background CDG (placeholder palette until the real stage palette is RE'd).
    let mut palette = [[0u8; 3]; 16];
    for (i, c) in palette.iter_mut().enumerate() {
        *c = [(i as u8) * 6, (i as u8) * 6, (i as u8) * 16];
    }
    let bg = arc.get(&std_name.replace(".STD", "BK.CDG")).and_then(|b| Cdg::parse(&b));
    let bg_wh = bg.as_ref().map(|c| (c.width as f32, c.height as f32));
    let bg_tex = match &bg {
        Some(c) => engine.create_texture(&c.decode_rgba(0, &palette).unwrap(), c.width as u32, c.height as u32),
        None => engine.create_texture(&[8, 8, 26, 255], 1, 1),
    };

    // Player sprite (Marisa, cel 0).
    let mari = arc.get("MARI.BFT").and_then(|b| Bft::parse(&b));
    let (player_tex, player_wh) = match &mari {
        Some(b) => (engine.create_texture(&b.decode_rgba(0, Some(0)).unwrap(), b.width as u32, b.height as u32), (b.width as f32, b.height as f32)),
        None => (engine.create_texture(&[102, 179, 255, 255], 1, 1), (16.0, 20.0)),
    };

    let mut textures: Vec<th06_engine::Texture> = vec![white, bg_tex, player_tex];
    // Load the sprite sheets into the global cel table at their PAT_* bases
    // (main_pat.h): shared sheets, then the stage sheet at PAT_STAGE = 128.
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

    // Dark playfield backdrop, then the background scrolled + tiled down.
    cmds.push(DrawCmd { tex: 0, dst: [PF_LEFT, PF_TOP, PF_W, PF_H], src: [0.0, 0.0, 1.0, 1.0], tint: [0.04, 0.04, 0.10, 1.0], rot: 0.0 });
    if let Some((bw, bh)) = bg_wh {
        let scroll = (until as f32 % bh) - bh; // background scrolls downward over time
        let mut y = scroll;
        while y < PF_H {
            cmds.push(DrawCmd { tex: 1, dst: [PF_LEFT, PF_TOP + y, bw, bh], src: [0.0, 0.0, 1.0, 1.0], tint: [1.0; 4], rot: 0.0 });
            y += bh;
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
        cmds.push(sprite(2, ppx, ppy, player_wh.0, player_wh.1));
    } else {
        cmds.push(solid(ppx, ppy, 16.0, 20.0, [0.4, 0.7, 1.0, 1.0]));
    }

    let texes: Vec<&th06_engine::Texture> = textures.iter().collect();
    let frame_img = engine.render_to_image(&cmds, &texes, None);
    image::save_buffer(&out, &frame_img, SCREEN_W, SCREEN_H, image::ColorType::Rgba8).expect("save png");
    println!(
        "frame {}: {} enemies, {} bullets, {} shots, score {} -> {}",
        until, sim.alive_enemies(), sim.bullets.active_count(), sim.player.active_shots(), sim.score, out
    );
}
