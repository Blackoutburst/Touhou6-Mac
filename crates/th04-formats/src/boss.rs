//! Boss framework, modelled on ReC98 `boss_stuff_t` (th04/main/boss/boss.hpp):
//! position, HP split into phases that each end at an HP threshold
//! (`boss_phase_next`), a defeat sequence, and `boss_hittest_shots`-style
//! damage. In the original each boss has a hardcoded per-frame update with its
//! own movement + spell patterns; those are a large per-boss reverse-
//! engineering task. This provides the faithful *structure* plus a generic
//! multi-phase behaviour (sway + alternating ring/aimed-spread attacks) so the
//! stage has a real boss fight; exact stage-1 patterns are TODO from the asm.
//!
//! Positions are subpixels (16/px).

use crate::bullet::{group, BulletPool, BulletTemplate};

const SUBPIXEL: i32 = 16;
const PLAYFIELD_W: i32 = 384;
/// Boss hitbox half-extent (ReC98 BOSS_HITBOX_DEFAULT ≈ 24px).
pub const BOSS_HIT: i32 = 24 * SUBPIXEL;
const BOSS_DEFEAT_FRAMES: u32 = 120;
const FIRE_INTERVAL: u32 = 48;

pub struct Boss {
    pub x: i32,
    pub y: i32,
    pub hp: i32,
    pub max_hp: i32,
    pub phase: u8,
    pub phase_count: u8,
    pub phase_frame: u32,
    pub defeated: bool,
    pub defeat_frame: u32,
    angle: u8,
    home_x: i32,
}

impl Boss {
    /// Spawn a boss with `max_hp` split across `phases` HP thresholds.
    pub fn new(max_hp: i32, phases: u8) -> Self {
        let home_x = (PLAYFIELD_W / 2) * SUBPIXEL;
        Boss {
            x: home_x,
            y: 80 * SUBPIXEL,
            hp: max_hp,
            max_hp,
            phase: 0,
            phase_count: phases.max(1),
            phase_frame: 0,
            defeated: false,
            defeat_frame: 0,
            angle: 0,
            home_x,
        }
    }

    /// HP threshold at which the current phase ends.
    fn phase_end_hp(&self) -> i32 {
        let remaining = (self.phase_count - 1 - self.phase.min(self.phase_count - 1)) as i32;
        self.max_hp * remaining / self.phase_count as i32
    }

    /// True once the defeat animation has finished (stage may advance).
    pub fn done(&self) -> bool {
        self.defeated && self.defeat_frame >= BOSS_DEFEAT_FRAMES
    }

    /// Apply player-shot damage; advances phases and triggers defeat.
    pub fn damage(&mut self, dmg: i32) {
        if self.defeated {
            return;
        }
        self.hp -= dmg;
        if self.hp <= self.phase_end_hp() {
            if (self.phase as u16 + 1) >= self.phase_count as u16 {
                self.defeated = true;
                self.defeat_frame = 0;
            } else {
                self.phase += 1;
                self.phase_frame = 0;
                // Clearing the screen happens in the sim.
            }
        }
    }

    /// Advance one frame: movement + attacks (fires into `pool`).
    pub fn update(&mut self, player: (i32, i32), pool: &mut BulletPool) {
        if self.defeated {
            self.defeat_frame += 1;
            return;
        }
        self.phase_frame += 1;
        // Sway horizontally around the home position.
        self.angle = self.angle.wrapping_add(1);
        let sway = (crate::math::cos8(self.angle) * (96 * SUBPIXEL)) >> 8;
        self.x = self.home_x + sway;

        // Attack: alternate patterns per phase on a fixed cadence.
        if self.phase_frame % FIRE_INTERVAL == 0 {
            let t = match self.phase % 3 {
                0 => BulletTemplate { group: group::RING_AIMED, count: 16, speed: 28, ..Default::default() },
                1 => BulletTemplate { group: group::SPREAD_AIMED, count: 5, speed: 32, delta: 10, ..Default::default() },
                _ => BulletTemplate { group: group::RING, count: 24, speed: 24, ..Default::default() },
            };
            pool.spawn(&t, self.x, self.y, player);
        }
    }
}

/// Stage-1 midboss, reverse-engineered from `midboss1_update` (th04_main.asm).
/// Phases: entrance (descend) → settle → attack. The attack is `sub_13FB2`:
/// every 8 frames fire a symmetric pair of blue Bullet16s at angle θ and
/// 128−θ, with θ stepping by 0x0C each shot (a sweeping spray, speed 2px). HP
/// 620 (the HUD bar maximum). It activates mid-stage and pauses the trash
/// timeline until defeated.
pub const MIDBOSS1_HP: i32 = 620;
const MIDBOSS_ENTRANCE_FRAMES: u32 = 96;
const MIDBOSS_TARGET_Y: i32 = 96 * SUBPIXEL;

pub struct Midboss {
    pub x: i32,
    pub y: i32,
    pub hp: i32,
    pub phase: u8, // 0 = entrance, 1 = attack
    pub phase_frame: u32,
    pub defeated: bool,
    pub defeat_frame: u32,
    sweep: u8, // byte_25594
}

impl Midboss {
    pub fn new(x: i32) -> Self {
        Midboss {
            x,
            y: -32 * SUBPIXEL,
            hp: MIDBOSS1_HP,
            phase: 0,
            phase_frame: 0,
            defeated: false,
            defeat_frame: 0,
            sweep: 1,
        }
    }

    pub fn done(&self) -> bool {
        self.defeated && self.defeat_frame >= BOSS_DEFEAT_FRAMES
    }

    pub fn damage(&mut self, dmg: i32) {
        if self.defeated || self.phase == 0 {
            return; // invulnerable during entrance
        }
        self.hp -= dmg;
        if self.hp <= 0 {
            self.defeated = true;
            self.defeat_frame = 0;
        }
    }

    pub fn update(&mut self, pool: &mut BulletPool) {
        if self.defeated {
            self.defeat_frame += 1;
            return;
        }
        self.phase_frame += 1;
        match self.phase {
            0 => {
                // Entrance: descend into position.
                self.y += 2 * SUBPIXEL;
                if self.y >= MIDBOSS_TARGET_Y && self.phase_frame >= MIDBOSS_ENTRANCE_FRAMES {
                    self.y = MIDBOSS_TARGET_Y;
                    self.phase = 1;
                    self.phase_frame = 0;
                    self.sweep = 1;
                }
            }
            _ => {
                // Attack: sub_13FB2 — symmetric sweeping pair every 8 frames.
                if self.phase_frame % 8 == 0 {
                    let t = BulletTemplate {
                        group: group::SINGLE,
                        count: 1,
                        speed: 2 * 16,
                        angle: self.sweep,
                        ..Default::default()
                    };
                    pool.spawn(&t, self.x, self.y - SUBPIXEL, (0, 0));
                    let mut t2 = t;
                    t2.angle = 0x80u8.wrapping_sub(self.sweep);
                    pool.spawn(&t2, self.x, self.y - SUBPIXEL, (0, 0));
                    self.sweep = self.sweep.wrapping_add(0x0c);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn midboss_entrance_then_defeat() {
        let mut m = Midboss::new(192 * 16);
        let mut pool = BulletPool::new();
        // Entrance: invulnerable.
        m.damage(100);
        assert_eq!(m.hp, MIDBOSS1_HP);
        for _ in 0..200 {
            m.update(&mut pool);
        }
        assert_eq!(m.phase, 1, "should reach the attack phase");
        assert!(pool.active_count() > 0, "should fire its sweep");
        m.hp = 1;
        m.damage(10);
        assert!(m.defeated);
    }

    #[test]
    fn phases_then_defeat() {
        let mut b = Boss::new(40, 4); // 4 phases, 10 HP each
        let mut pool = BulletPool::new();
        for _ in 0..60 {
            b.damage(1);
            b.update((0, 0), &mut pool);
            if b.defeated {
                break;
            }
        }
        assert!(b.defeated, "boss should be defeated after enough damage");
        assert!(b.phase >= 3, "should have advanced through phases");
        for _ in 0..(BOSS_DEFEAT_FRAMES + 1) {
            b.update((0, 0), &mut pool);
        }
        assert!(b.done(), "defeat animation should finish");
    }

    #[test]
    fn fires_bullets() {
        let mut b = Boss::new(1000, 4);
        let mut pool = BulletPool::new();
        for _ in 0..FIRE_INTERVAL {
            b.update((100, 100), &mut pool);
        }
        assert!(pool.active_count() > 0, "boss should fire a pattern");
    }
}
