//! Decode a PI image from a TH04 archive to a PNG (palette is embedded).
//!
//!   cargo run -p th04-formats --example pi2png -- <archive> <NAME.PI> [out.png]

use th04_formats::par::Archive;
use th04_formats::pi::Pi;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() < 2 {
        eprintln!("usage: pi2png <archive> <NAME.PI> [out.png]");
        std::process::exit(2);
    }
    let arc = Archive::parse(std::fs::read(&a[0]).expect("read archive")).expect("parse archive");
    let raw = arc.get(&a[1]).unwrap_or_else(|| panic!("member {} not found", a[1]));
    let pi = Pi::parse(&raw).expect("parse PI");
    let out = a.get(2).cloned().unwrap_or_else(|| "pi.png".into());
    println!("{}: {}x{}", a[1], pi.width, pi.height);
    image::save_buffer(
        &out,
        &pi.to_rgba(),
        pi.width as u32,
        pi.height as u32,
        image::ColorType::Rgba8,
    )
    .expect("save png");
    println!("wrote {}", out);
}
