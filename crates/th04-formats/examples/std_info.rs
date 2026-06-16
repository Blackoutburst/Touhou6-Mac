//! Dump a TH04 stage (.STD): map scroll order, scroll speeds, enemy scripts,
//! and stage-timeline size.
//!
//!   cargo run -p th04-formats --example std_info -- <archive> <STnn.STD>

use th04_formats::par::Archive;
use th04_formats::stage::Std;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() < 2 {
        eprintln!("usage: std_info <archive> <STnn.STD>");
        std::process::exit(2);
    }
    let arc = Archive::parse(std::fs::read(&a[0]).expect("read archive")).expect("parse archive");
    let raw = arc.get(&a[1]).unwrap_or_else(|| panic!("{} not found", a[1]));
    let s = Std::parse(&raw).expect("parse STD");

    println!("{}", a[1]);
    println!("  map sections : {} -> {:?}", s.map_section_order.len(), s.map_section_order);
    println!("  scroll speeds: {} -> {:?}", s.scroll_speeds.len(), s.scroll_speeds);
    println!("  enemy scripts: {}", s.enemy_scripts.len());
    for (i, e) in s.enemy_scripts.iter().enumerate() {
        println!("     #{:<2} {} bytes", i, e.len());
    }
    let events = s.timeline_events();
    let spawns: usize = events.iter().map(|e| e.spawns.len()).sum();
    println!("  timeline     : {} bytes, {} spawn-frames, {} enemies", s.timeline.len(), events.len(), spawns);
    for e in events.iter().take(8) {
        let s0 = e.spawns.first();
        println!(
            "     frame {:>5}: {} enemy(s){}",
            e.frame,
            e.spawns.len(),
            s0.map(|s| format!("  e.g. script#{} @({},{}) arg={}", s.script_index, s.x, s.y, s.arg)).unwrap_or_default()
        );
    }
}
