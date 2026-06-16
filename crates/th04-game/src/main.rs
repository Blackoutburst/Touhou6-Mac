//! TH04 game harness — first milestone: render a PI image (the title screen)
//! through the shared `th06-engine` wgpu renderer.
//!
//!   th04-game <archive> [NAME.PI] [out.png]
//!
//! Defaults to OP1.PI (the title). Renders offscreen via `render_to_image` so
//! it can be verified headlessly; the same texture + DrawCmd path drives the
//! live window once we add interactivity.

use th04_formats::par::Archive;
use th04_formats::pi::Pi;
use th06_engine::{DrawCmd, Engine, SCREEN_H, SCREEN_W};

fn main() {
    let mut args = std::env::args().skip(1);
    let archive = args
        .next()
        .expect("usage: th04-game <archive> [NAME.PI] [out.png]");
    let name = args.next().unwrap_or_else(|| "OP1.PI".into());
    let out = args.next().unwrap_or_else(|| "title.png".into());

    let arc = Archive::parse(std::fs::read(&archive).expect("read archive")).expect("parse archive");
    let raw = arc
        .get(&name)
        .unwrap_or_else(|| panic!("{} not in archive", name));
    let pi = Pi::parse(&raw).expect("parse PI");
    let rgba = pi.to_rgba();

    let engine = Engine::new();
    let tex = engine.create_texture(&rgba, pi.width as u32, pi.height as u32);

    // Center the image (PI title is 640x400) within the 640x480 frame.
    let x = (SCREEN_W as f32 - pi.width as f32) / 2.0;
    let y = (SCREEN_H as f32 - pi.height as f32) / 2.0;
    let cmd = DrawCmd {
        tex: 0,
        dst: [x, y, pi.width as f32, pi.height as f32],
        src: [0.0, 0.0, 1.0, 1.0],
        tint: [1.0, 1.0, 1.0, 1.0],
        rot: 0.0,
    };

    let frame = engine.render_to_image(&[cmd], &[&tex], None);
    image::save_buffer(&out, &frame, SCREEN_W, SCREEN_H, image::ColorType::Rgba8).expect("save png");
    println!("rendered {} ({}x{}) -> {}", name, pi.width, pi.height, out);
}
