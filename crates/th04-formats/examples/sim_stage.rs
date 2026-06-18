//! Headlessly simulate a whole TH04 stage: spawn timeline + enemy VM + bullets
//! + player + collision. Holds "shoot" down and weaves the player; reports the
//! running state.
//!
//!   cargo run -p th04-formats --example sim_stage -- <archive> <STnn.STD> [frames]

use th04_formats::boss::BossKind;
use th04_formats::par::Archive;
use th04_formats::player::Input;
use th04_formats::sim::StageSim;
use th04_formats::stage::Std;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() < 2 {
        eprintln!("usage: sim_stage <archive> <STnn.STD> [frames]");
        std::process::exit(2);
    }
    let arc = Archive::parse(std::fs::read(&a[0]).expect("read archive")).expect("parse archive");
    let std = Std::parse(&arc.get(&a[1]).unwrap()).expect("parse STD");
    let frames: u32 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(2000);

    // Stage index from the STD name (ST00 → 0); shot_type 0 = Reimu → Marisa rival.
    let stage_idx: usize = a[1]
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .unwrap_or(0);
    let mut sim = StageSim::new(std, 0, BossKind::for_stage(stage_idx, false));
    println!("simulating {} for {} frames (shoot held)\n", a[1], frames);
    println!("  frame  alive  spawned  killed   score  bullets  pshots  hits  lives");
    for f in 0..frames {
        let mut input = Input::default();
        input.shoot = true;
        // gentle weave so the player isn't a sitting duck
        input.left = (f / 64) % 2 == 0;
        input.right = !input.left;
        sim.step(&input);
        if f % 200 == 0 {
            println!(
                "  {:>5}  {:>5}  {:>7}  {:>6}  {:>6}  {:>7}  {:>6}  {:>4}  {:>5}",
                f,
                sim.alive_enemies(),
                sim.enemies_spawned,
                sim.enemies_killed,
                sim.score,
                sim.bullets.active_count(),
                sim.player.active_shots(),
                sim.player_hits,
                sim.player.lives,
            );
        }
    }
    println!(
        "\nfinal: spawned={} killed={} score={} player_hits={} lives={} gameover={} finished={}",
        sim.enemies_spawned, sim.enemies_killed, sim.score, sim.player_hits,
        sim.player.lives, sim.player.gameover, sim.finished()
    );
}
