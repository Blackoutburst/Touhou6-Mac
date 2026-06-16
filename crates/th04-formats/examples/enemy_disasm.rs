//! Disassemble all enemy scripts of a TH04 stage.
//!
//!   cargo run -p th04-formats --example enemy_disasm -- <archive> <STnn.STD>

use th04_formats::enemy;
use th04_formats::par::Archive;
use th04_formats::stage::Std;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() < 2 {
        eprintln!("usage: enemy_disasm <archive> <STnn.STD>");
        std::process::exit(2);
    }
    let arc = Archive::parse(std::fs::read(&a[0]).expect("read archive")).expect("parse archive");
    let raw = arc.get(&a[1]).unwrap_or_else(|| panic!("{} not found", a[1]));
    let s = Std::parse(&raw).expect("parse STD");

    let mut clean = 0;
    for (i, script) in s.enemy_scripts.iter().enumerate() {
        let (insns, complete) = enemy::disassemble(script);
        clean += complete as usize;
        let line: Vec<String> = insns
            .iter()
            .map(|n| {
                if n.operands.is_empty() {
                    n.mnemonic.to_string()
                } else {
                    format!("{}({})", n.mnemonic, n.operands.iter().map(|b| format!("{b}")).collect::<Vec<_>>().join(","))
                }
            })
            .collect();
        println!("#{:<2} ({:>3}b){}: {}", i, script.len(), if complete { " " } else { "!" }, line.join(" "));
    }
    println!("\n{}/{} scripts terminate cleanly", clean, s.enemy_scripts.len());
}
