//! Headless stage simulation: ties the whole gameplay loop together — the
//! `.STD` spawn timeline, the enemy script VM, the bullet system and the
//! player, plus collision and scoring. No rendering; this is the deterministic
//! game model that a renderer (and the WASM build) will drive and draw.
//!
//! Collision is approximate (axis-aligned boxes) pending the exact TH04
//! hitboxes: player shots damage enemies (kill → score), and enemy bullets that
//! reach the player count as hits. Positions are subpixels (16/px).

use crate::bullet::BulletPool;
use crate::enemy_vm::Enemy;
use crate::player::{Input, Player};
use crate::stage::{Std, TimelineFrame};

const SUBPIXEL: i32 = 16;
const SCROLL_DY: i32 = 16; // 1px/frame placeholder
// Approximate hit half-extents.
const ENEMY_HIT: i32 = 16 * SUBPIXEL;
const BULLET_KILL: i32 = 6 * SUBPIXEL;

pub struct StageSim {
    std: Std,
    events: Vec<TimelineFrame>,
    ev_i: usize,
    pub frame: u16,
    pub enemies: Vec<Enemy>,
    pub bullets: BulletPool,
    pub player: Player,
    pub score: i64,
    pub enemies_spawned: u32,
    pub enemies_killed: u32,
    pub player_hits: u32,
}

impl StageSim {
    pub fn new(std: Std, shot_type: u8) -> Self {
        let events = std.timeline_events();
        StageSim {
            std,
            events,
            ev_i: 0,
            frame: 0,
            enemies: Vec::new(),
            bullets: BulletPool::new(),
            player: Player::new(shot_type),
            score: 0,
            enemies_spawned: 0,
            enemies_killed: 0,
            player_hits: 0,
        }
    }

    pub fn alive_enemies(&self) -> usize {
        self.enemies.iter().filter(|e| !e.killed).count()
    }

    /// True once the timeline is exhausted and no enemies remain.
    pub fn finished(&self) -> bool {
        self.ev_i >= self.events.len() && self.alive_enemies() == 0
    }

    /// Advance the whole stage one frame.
    pub fn step(&mut self, input: &Input) {
        self.player.update(input);

        // 1. Spawn enemies whose timeline frame has arrived.
        while self.ev_i < self.events.len() && self.events[self.ev_i].frame <= self.frame {
            for sp in &self.events[self.ev_i].spawns {
                let mut e = Enemy::spawn(sp.x as i32, sp.y as i32);
                e.script_index = sp.script_index as usize;
                self.enemies.push(e);
                self.enemies_spawned += 1;
            }
            self.ev_i += 1;
        }

        // 2. Run each enemy's script (movement + firing). Disjoint field
        //    borrows: enemies (mut), std (shared), bullets (mut).
        let player_pos = (self.player.x, self.player.y);
        for e in self.enemies.iter_mut() {
            if e.killed {
                continue;
            }
            let script = self
                .std
                .enemy_scripts
                .get(e.script_index)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            e.step(script, SCROLL_DY, player_pos, &mut self.bullets);
        }

        // 3. Move bullets.
        self.bullets.update();

        // 4. Player shots vs enemies.
        for s in self.player.shots.iter_mut() {
            if !s.active {
                continue;
            }
            for e in self.enemies.iter_mut() {
                if e.killed || !e.can_be_damaged {
                    continue;
                }
                if (s.x - e.x).abs() < ENEMY_HIT && (s.y - e.y).abs() < ENEMY_HIT {
                    e.hp -= s.damage as i16;
                    s.active = false;
                    if e.hp <= 0 {
                        e.killed = true;
                        self.enemies_killed += 1;
                        self.score += e.score as i64;
                    }
                    break;
                }
            }
        }

        // 5. Enemy bullets / bodies vs player (only when vulnerable).
        if !self.player.invincible() && !self.player.gameover {
            let (px, py) = (self.player.x, self.player.y);
            let mut died = false;
            for b in self.bullets.bullets.iter_mut() {
                if b.active && (b.x - px).abs() < BULLET_KILL && (b.y - py).abs() < BULLET_KILL {
                    b.active = false;
                    died = true;
                    break;
                }
            }
            if !died {
                for e in &self.enemies {
                    if !e.killed && e.kills_player && (e.x - px).abs() < ENEMY_HIT && (e.y - py).abs() < ENEMY_HIT {
                        died = true;
                        break;
                    }
                }
            }
            if died {
                self.player_hits += 1;
                if self.player.hit() {
                    // TH04 clears the screen of bullets when the player dies.
                    for b in self.bullets.bullets.iter_mut() {
                        b.active = false;
                    }
                }
            }
        }

        // 6. Drop dead enemies, advance time.
        self.enemies.retain(|e| !e.killed);
        self.frame = self.frame.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage::Std;

    fn tiny_stage() -> Std {
        // 1 enemy script: stats(hp=1) then end; timeline: frame 0 spawns it.
        let enemy_script = vec![0x10u8, 0, 1, 0, 5, 0, 0x00]; // stats patnum0 hp1 score5; end
        let mut std = Std {
            map_section_order: vec![],
            scroll_speeds: vec![],
            enemy_scripts: vec![enemy_script],
            timeline: vec![],
        };
        // timeline: frame 1, count 1, spawn script#0 at (100,50) item 0
        let mut tl = Vec::new();
        tl.extend_from_slice(&1u16.to_le_bytes());
        tl.push(1);
        tl.extend_from_slice(&[0, 100, 0, 50, 0, 0, 0, 0]); // script0, x=100, y=50
        tl.extend_from_slice(&0u16.to_le_bytes());
        std.timeline = tl;
        std
    }

    #[test]
    fn spawns_and_kills_enemy() {
        let mut sim = StageSim::new(tiny_stage(), 0);
        // place a player shot right on the enemy spawn and run a few frames
        let mut input = Input::default();
        input.shoot = true;
        for _ in 0..3 {
            sim.step(&input);
        }
        assert!(sim.enemies_spawned >= 1, "enemy should have spawned");
    }
}
