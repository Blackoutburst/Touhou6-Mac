//! TH04 native harness:
//!   th04-game title <archive> [out.png]                 — render the title
//!   th04-game stage <archive> <STnn.STD> [frame|boss|midboss] [out.png]
//!       — run the sim and render one frame offscreen (verification)
//!   th04-game play  <archive> <STnn.STD>                 — play in a window
//!
//! The `play` loop and the WASM build (lib::web) share lib::setup / draw_frame.

use th04_formats::boss::{Boss, BossKind, Midboss};
use th04_formats::par::Archive;
use th04_formats::pi::Pi;
use th04_formats::player::Input;
use th04_formats::sim::Phase;
use th04_game::{draw_frame, make_update, setup};
use th06_engine::{DrawCmd, Engine, SCREEN_H, SCREEN_W};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("play") => play(&args[1..]),
        Some("stage") => stage(&args[1..]),
        _ => title(&args[1..]),
    }
}

fn read_archive(path: &str) -> Archive {
    Archive::parse(std::fs::read(path).expect("read archive")).expect("parse archive")
}

fn title(a: &[String]) {
    let archive = a.first().expect("usage: th04-game title <archive> [out.png]");
    let out = a.get(1).cloned().unwrap_or_else(|| "title.png".into());
    let arc = read_archive(archive);
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

/// Interactive windowed play (native).
fn play(a: &[String]) {
    let arc = read_archive(a.first().expect("usage: th04-game play <archive> <STnn.STD>"));
    let std_name = a.get(1).map(String::as_str).unwrap_or("ST00.STD");
    let engine = Engine::new();
    let (textures, dd, sim) = setup(&engine, &arc, std_name, 2 /* Marisa */);
    engine.run_game("Touhou 4 ~ Lotus Land Story", textures, make_update(sim, dd));
}

/// Offscreen single-frame render for verification.
fn stage(a: &[String]) {
    if a.len() < 2 {
        eprintln!("usage: th04-game stage <archive> <STnn.STD> [frame|boss|midboss] [out.png]");
        std::process::exit(2);
    }
    let arc = read_archive(&a[0]);
    let std_name = &a[1];
    let until: u32 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(300);
    let out = a.get(3).cloned().unwrap_or_else(|| "stage.png".into());
    // `boss` (= this stage's boss) or `boss:<name>` to force a specific one.
    let boss_arg = a.get(2).map(String::as_str).unwrap_or("");
    let force_boss = boss_arg == "boss" || boss_arg.starts_with("boss:");
    let force_midboss = boss_arg == "midboss";
    // Default to the stage's own boss (Reimu's player at the rival fight); an
    // explicit `boss:<name>` overrides it.
    let stage_boss = th04_game::stage_index(std_name);
    let default_boss = BossKind::for_stage(stage_boss, false).unwrap_or(BossKind::Orange);
    let boss_name = boss_arg.strip_prefix("boss:").map(str::to_string)
        .unwrap_or_else(|| format!("{:?}", default_boss).to_lowercase());

    let engine = Engine::new();
    let (textures, dd, mut sim) = setup(&engine, &arc, std_name, 2);
    for f in 0..until {
        let mut input = Input::default();
        input.shoot = true;
        input.left = (f / 64) % 2 == 0;
        input.right = !input.left;
        sim.step(&input);
    }
    if force_boss || force_midboss {
        sim.player.gameover = false;
        sim.player.lives = 2;
        if force_boss {
            sim.phase = Phase::Boss;
            let make = match boss_name.as_str() {
                "kurumi" => Boss::kurumi,
                "elly" => Boss::elly,
                "reimu" => Boss::reimu,
                "marisa" => Boss::marisa,
                "yuuka" => Boss::yuuka,
                "yuuka6" => Boss::yuuka6,
                _ => Boss::orange,
            };
            sim.boss.get_or_insert_with(make);
        } else {
            sim.midboss.get_or_insert_with(|| Midboss::new(192 * 16));
        }
        // Run past the boss intro, then keep going until a frame actually shows
        // danmaku (so the screenshot is representative), capped well above any
        // intro length.
        let min_frames = if force_boss { 360 } else { 160 };
        for f in 0..900 {
            let mut input = Input::default();
            input.shoot = true;
            sim.step(&input);
            if f >= min_frames && sim.bullets.active_count() >= 12 {
                break;
            }
        }
    }

    let cmds = draw_frame(&sim, &dd);
    let texes: Vec<&th06_engine::Texture> = textures.iter().collect();
    let frame_img = engine.render_to_image(&cmds, &texes, None);
    image::save_buffer(&out, &frame_img, SCREEN_W, SCREEN_H, image::ColorType::Rgba8).expect("save png");
    println!(
        "{:?} frame {}: {} enemies, {} bullets, boss_hp {:?} -> {}",
        sim.phase, until, sim.alive_enemies(), sim.bullets.active_count(),
        sim.boss.as_ref().map(|b| b.hp), out
    );
}
