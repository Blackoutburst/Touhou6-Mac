//! TH04 native harness:
//!   th04-game menu  <archive>                            — title → menu → play
//!   th04-game title <archive> [out.png]                 — render the title
//!   th04-game stage <archive> <STnn.STD> [frame|boss|midboss] [out.png]
//!       — run the sim and render one frame offscreen (verification)
//!   th04-game play  <archive> <STnn.STD>                 — play a stage directly
//!
//! The `play`/`menu` loops and the WASM build (lib::web) share lib::setup /
//! lib::setup_menu / draw_frame.

use std::path::Path;

use th04_formats::boss::{Boss, BossKind, Midboss};
use th04_formats::par::Archive;
use th04_formats::pi::Pi;
use th04_formats::player::Input;
use th04_formats::sim::Phase;
use th04_game::menu::make_menu_update;
use th04_game::{draw_frame, make_update, setup, setup_menu};
use th06_engine::{DrawCmd, Engine, SCREEN_H, SCREEN_W};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("menu") => menu(&args[1..]),
        Some("menushot") => menushot(&args[1..]),
        Some("sheet") => sheet(&args[1..]),
        Some("cd2") => cd2(&args[1..]),
        Some("play") => play(&args[1..]),
        Some("stage") => stage(&args[1..]),
        _ => title(&args[1..]),
    }
}

fn read_archive(path: &str) -> Archive {
    Archive::parse(std::fs::read(path).expect("read archive")).expect("parse archive")
}

/// Decode the title art (`OP1.PI`). It ships in `幻想郷ED.DAT` (the menu/OP
/// archive), so we look there first — derived from the main archive's directory
/// — then fall back to the main archive, then to a text-only title.
fn load_title_image(main_path: &str, main_arc: &Archive) -> Option<(Vec<u8>, u32, u32)> {
    let ed_bytes = Path::new(main_path)
        .parent()
        .map(|d| d.join("幻想郷ED.DAT"))
        .and_then(|p| std::fs::read(p).ok());
    let ed_arc = ed_bytes.and_then(|b| Archive::parse(b).ok());
    let pi_bytes = ed_arc
        .as_ref()
        .and_then(|a| a.get("OP1.PI"))
        .or_else(|| main_arc.get("OP1.PI"))?;
    let pi = Pi::parse(&pi_bytes)?;
    Some((pi.to_rgba(), pi.width as u32, pi.height as u32))
}

/// Title → menu → play, in a window.
fn menu(a: &[String]) {
    let path = a.first().expect("usage: th04-game menu <archive>");
    let arc = read_archive(path);
    let title_img = load_title_image(path, &arc);
    let engine = Engine::new();
    let (textures, app) = setup_menu(&engine, &arc, title_img);
    engine.run_game("Touhou 4 ~ Lotus Land Story", textures, make_menu_update(app));
}

/// Offscreen verification of the menu: drive it with synthetic key presses
/// through the title, main menu, the practice path (character → shot → rank →
/// stage select) and into a non-stage-1 stage, saving a PNG of each screen.
/// `th04-game menushot <archive> [out_prefix]`.
fn menushot(a: &[String]) {
    use th06_engine::{Input as EInput, Key};
    let path = a.first().expect("usage: th04-game menushot <archive> [prefix]");
    let prefix = a.get(1).cloned().unwrap_or_else(|| "menu".into());
    let arc = read_archive(path);
    let title_img = load_title_image(path, &arc);
    let engine = Engine::new();
    let (textures, app) = setup_menu(&engine, &arc, title_img);
    let texes: Vec<&th06_engine::Texture> = textures.iter().collect();
    let mut update = make_menu_update(app);
    let none = EInput::default();
    let press = |k: Key| EInput::synthetic(&[], &[k]);
    let enter = || EInput::synthetic(&[], &[Key::Shoot]);

    let save = |frame: &th06_engine::Frame, engine: &Engine, name: &str| {
        let img = engine.render_to_image(&frame.cmds, &texes, None);
        image::save_buffer(name, &img, SCREEN_W, SCREEN_H, image::ColorType::Rgba8).expect("save");
        println!("wrote {}", name);
    };

    // Title → Main (now showing the enabled PRACTICE/EXTRA/OPTION entries).
    save(&update(&none), &engine, &format!("{prefix}_0title.png"));
    update(&enter());
    save(&update(&none), &engine, &format!("{prefix}_1main.png"));
    // OPTION (4 downs to reach it), bump START LIVES (right), then back to Main.
    for _ in 0..4 {
        update(&press(Key::Down));
    }
    update(&enter());
    update(&press(Key::Right)); // START LIVES 2 → 3
    save(&update(&none), &engine, &format!("{prefix}_1opt.png"));
    update(&press(Key::Pause)); // back to Main (cursor 0)
    update(&none);
    // Pick PRACTICE START (one down), then Character / Shot / Rank.
    update(&press(Key::Down));
    update(&enter());
    save(&update(&none), &engine, &format!("{prefix}_2char.png"));
    update(&enter());
    save(&update(&none), &engine, &format!("{prefix}_3shot.png"));
    update(&enter());
    save(&update(&none), &engine, &format!("{prefix}_4rank.png"));
    // → Stage select; pick stage 3 (two downs) to prove non-stage-1 textures.
    update(&enter());
    save(&update(&none), &engine, &format!("{prefix}_5stage.png"));
    update(&press(Key::Down));
    update(&press(Key::Down));
    update(&enter());
    let mut f = update(&none);
    for _ in 0..120 {
        f = update(&EInput::synthetic(&[Key::Shoot], &[]));
    }
    save(&f, &engine, &format!("{prefix}_6play.png"));
}

/// Dump every cel of a `.BFT` sprite sheet into one PNG grid (magenta = the
/// transparent index) so we can see what each sheet contains.
/// `th04-game sheet <archive> <NAME.BFT> [out.png]`.
fn sheet(a: &[String]) {
    use th04_formats::bft::Bft;
    let arc = read_archive(a.first().expect("usage: th04-game sheet <archive> <NAME.BFT> [out.png]"));
    let name = a.get(1).expect("sheet name");
    let out = a.get(2).cloned().unwrap_or_else(|| "sheet.png".into());
    let scale: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(1);
    let cols_arg: usize = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(16);
    let b = arc.get(name).and_then(|d| Bft::parse(&d)).expect("parse BFT");
    let (cw, ch) = (b.width + 2, b.height + 2);
    let cols = b.count.min(cols_arg).max(1);
    let rows = b.count.div_ceil(cols);
    let (iw, ih) = (cols * cw, rows * ch);
    let mut img = vec![0u8; iw * ih * 4];
    for p in img.chunks_exact_mut(4) {
        p.copy_from_slice(&[255, 0, 255, 255]); // magenta backdrop
    }
    for n in 0..b.count {
        let Some(rgba) = b.decode_rgba(n, Some(0)) else { continue };
        let (ox, oy) = ((n % cols) * cw + 1, (n / cols) * ch + 1);
        for y in 0..b.height {
            for x in 0..b.width {
                let s = (y * b.width + x) * 4;
                if rgba[s + 3] == 0 {
                    continue;
                }
                let d = ((oy + y) * iw + ox + x) * 4;
                img[d..d + 4].copy_from_slice(&rgba[s..s + 4]);
            }
        }
    }
    // Optional nearest-neighbour upscale so individual cels are readable.
    let (fw, fh, fimg) = if scale > 1 {
        let (sw, sh) = (iw * scale, ih * scale);
        let mut up = vec![0u8; sw * sh * 4];
        for y in 0..sh {
            for x in 0..sw {
                let s = ((y / scale) * iw + x / scale) * 4;
                let d = (y * sw + x) * 4;
                up[d..d + 4].copy_from_slice(&img[s..s + 4]);
            }
        }
        (sw, sh, up)
    } else {
        (iw, ih, img)
    };
    image::save_buffer(&out, &fimg, fw as u32, fh as u32, image::ColorType::Rgba8).expect("save");
    println!("{}: {} cels {}x{} (start {}) -> {} ({}x)", name, b.count, b.width, b.height, b.start, out, scale);
}

/// Dump a `.CD2`/`.CDG` image (all frames) using a chosen palette source
/// (an `.MPN` stage palette or an `.RGB` file) to a PNG, so we can see whether
/// the stage palette renders the boss sprites correctly.
/// `th04-game cd2 <archive> <NAME.CD2> <PAL.MPN|PAL.RGB> [out.png]`.
fn cd2(a: &[String]) {
    use th04_formats::cdg::{parse_palette, Cdg, PALETTE_LEN};
    use th04_formats::mpn::Mpn;
    let arc = read_archive(a.first().expect("usage: th04-game cd2 <archive> <NAME.CD2> <PAL> [out.png]"));
    let name = a.get(1).expect("cd2 name");
    let pal_src = a.get(2).expect("palette source (.MPN or .RGB)");
    let out = a.get(3).cloned().unwrap_or_else(|| "cd2.png".into());
    let cd = arc.get(name).and_then(|d| Cdg::parse(&d)).expect("parse CD2");
    let pal: [[u8; 3]; PALETTE_LEN] = if pal_src.to_uppercase().ends_with(".MPN") {
        arc.get(pal_src).and_then(|d| Mpn::parse(&d)).expect("parse MPN").palette
    } else {
        parse_palette(&arc.get(pal_src).expect("read palette"))
    };
    let (w, h, n) = (cd.width, cd.height, cd.image_count);
    let (iw, ih) = ((w + 2) * n.min(8).max(1), (h + 2) * n.div_ceil(8));
    let mut img = vec![255u8; iw * ih * 4];
    for p in img.chunks_exact_mut(4) {
        p.copy_from_slice(&[255, 0, 255, 255]);
    }
    for i in 0..n {
        let Some(rgba) = cd.decode_rgba(i, &pal) else { continue };
        let (ox, oy) = ((i % 8) * (w + 2) + 1, (i / 8) * (h + 2) + 1);
        for y in 0..h {
            for x in 0..w {
                let s = (y * w + x) * 4;
                if rgba[s + 3] == 0 {
                    continue;
                }
                let d = ((oy + y) * iw + ox + x) * 4;
                img[d..d + 4].copy_from_slice(&rgba[s..s + 4]);
            }
        }
    }
    image::save_buffer(&out, &img, iw as u32, ih as u32, image::ColorType::Rgba8).expect("save");
    println!("{}: {} imgs {}x{} (pal {}) -> {}", name, n, w, h, pal_src, out);
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

    // Optional difficulty keyword anywhere in the args (default Normal).
    let rank = a.iter().find_map(|s| match s.as_str() {
        "easy" => Some(0u8),
        "normal" => Some(1),
        "hard" => Some(2),
        "lunatic" => Some(3),
        _ => None,
    });

    let engine = Engine::new();
    let (textures, dd, mut sim) = setup(&engine, &arc, std_name, 2);
    if let Some(r) = rank {
        sim.set_rank(r);
    }
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
