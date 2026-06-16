//! Unpack a TH04 archive (the file ZUN named `東方幻想.郷`, extracted from the
//! game's .hdi with `tools/pc98_hdi_extract.py`).
//!
//!   cargo run -p th04-formats --example unpack -- <archive> [outdir]
//!
//! Lists every member and, if `outdir` is given, writes each decompressed file
//! there. Validates that every member decodes to exactly its recorded size.

use std::path::PathBuf;
use th04_formats::par::Archive;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = match args.next() {
        Some(p) => p,
        None => {
            eprintln!("usage: unpack <archive> [outdir]");
            std::process::exit(2);
        }
    };
    let outdir = args.next().map(PathBuf::from);

    let data = std::fs::read(&path).expect("read archive");
    let arc = Archive::parse(data).expect("parse archive");
    println!("{} members (key {:#04x})\n", arc.entries.len(), arc.key);

    let mut ok = 0usize;
    if let Some(dir) = &outdir {
        std::fs::create_dir_all(dir).expect("mkdir outdir");
    }
    for e in &arc.entries {
        let bytes = arc.extract(e);
        let good = bytes.len() as u32 == e.orig_size;
        ok += good as usize;
        println!(
            "  {:<12} {:<4} {:>7} -> {:>7}  {}",
            e.name,
            if e.compressed { "rle" } else { "raw" },
            e.packed_size,
            e.orig_size,
            if good { "ok" } else { "SIZE MISMATCH" },
        );
        if let Some(dir) = &outdir {
            std::fs::write(dir.join(&e.name), &bytes).expect("write member");
        }
    }
    println!("\n{}/{} members decoded to exact size", ok, arc.entries.len());
}
