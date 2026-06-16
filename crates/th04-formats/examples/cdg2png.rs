//! Decode a CDG/CD2 image from a TH04 archive to a PNG, to eyeball the planar
//! decoder.
//!
//!   cargo run -p th04-formats --example cdg2png -- <archive> <NAME.CDG> [PALETTE.RGB] [out.png] [image_index]
//!
//! If no palette is given, a grayscale ramp over the 16 indices is used (enough
//! to confirm the planar de-interleave is correct, independent of colour).

use th04_formats::cdg::{self, Cdg, PALETTE_LEN};
use th04_formats::par::Archive;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() < 2 {
        eprintln!("usage: cdg2png <archive> <NAME.CDG> [PALETTE.RGB] [out.png] [image_index]");
        std::process::exit(2);
    }
    let arc = Archive::parse(std::fs::read(&a[0]).expect("read archive")).expect("parse archive");
    let cdg_bytes = arc.get(&a[1]).unwrap_or_else(|| panic!("member {} not found", a[1]));
    let cdg = Cdg::parse(&cdg_bytes).expect("parse CDG");

    let palette = match a.get(2) {
        Some(p) => cdg::parse_palette(&arc.get(p).unwrap_or_else(|| panic!("palette {} not found", p))),
        None => {
            let mut pal = [[0u8; 3]; PALETTE_LEN];
            for (i, c) in pal.iter_mut().enumerate() {
                let v = (i * 17) as u8;
                *c = [v, v, v];
            }
            pal
        }
    };
    let out = a.get(3).cloned().unwrap_or_else(|| "cdg.png".into());
    let n: usize = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(0);

    println!(
        "{}: {}x{} {} image(s), layout {:?}",
        a[1], cdg.width, cdg.height, cdg.image_count, cdg.plane_layout
    );
    let rgba = cdg.decode_rgba(n, &palette).expect("decode image");
    image::save_buffer(
        &out,
        &rgba,
        cdg.width as u32,
        cdg.height as u32,
        image::ColorType::Rgba8,
    )
    .expect("save png");
    println!("wrote {}", out);
}
