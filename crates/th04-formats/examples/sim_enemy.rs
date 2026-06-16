//! Simulate one enemy script for N frames and print its trajectory.
//!
//!   cargo run -p th04-formats --example sim_enemy -- <archive> <STnn.STD> <script#> [frames]

use th04_formats::bullet::BulletPool;
use th04_formats::enemy_vm::Enemy;
use th04_formats::par::Archive;
use th04_formats::stage::Std;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() < 3 {
        eprintln!("usage: sim_enemy <archive> <STnn.STD> <script#> [frames]");
        std::process::exit(2);
    }
    let arc = Archive::parse(std::fs::read(&a[0]).expect("read archive")).expect("parse archive");
    let std = Std::parse(&arc.get(&a[1]).unwrap()).expect("parse STD");
    let idx: usize = a[2].parse().unwrap();
    let frames: u32 = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(360);
    let script = &std.enemy_scripts[idx];

    // Spawn at horizontal centre, just above the playfield (subpixels).
    let mut e = Enemy::spawn(192 * 16, -16 * 16);
    let mut pool = BulletPool::new();
    let player = (192 * 16, 400 * 16);
    println!(
        "sim {} #{} ({} bytes) for {} frames",
        a[1], idx, script.len(), frames
    );
    for f in 0..frames {
        e.step(script, 16 /* scroll 1px/frame */, player, &mut pool);
        pool.update();
        if f % 30 == 0 || e.killed {
            println!(
                "  f{:>4}: pos=({:>4},{:>4})px angle={:>3} speed={:>3} hp={} fires={} bullets={} alive={} killed={}",
                f, e.x / 16, e.y / 16, e.angle, e.speed, e.hp, e.fire_count, pool.active_count(), e.alive, e.killed
            );
        }
        if e.killed {
            break;
        }
    }
}
