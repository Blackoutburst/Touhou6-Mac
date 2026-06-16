//! TH04 bullet system: spawn bullets from a [`BulletTemplate`] according to its
//! group pattern, and move them each frame. Group semantics follow ReC98
//! `th04/main/bullet/types.h`; movement follows `bullet_velocity_set_from_angle_
//! and_speed` (regular bullets fly straight at `vector2(angle, speed)`).
//!
//! Special motions (`BSM_*`: bounce, gravity, decelerate-then-turn) and the
//! decelerate-from-base-speed ramp are not yet modelled — stage trash fires
//! regular bullets, which this covers. Positions are subpixels (16/px).

use crate::math::{iatan2, vector2};

const SUBPIXEL: i32 = 16;
// TODO: confirm exact TH04 playfield; only affects off-screen culling.
const PLAYFIELD_W: i32 = 384;
const PLAYFIELD_H: i32 = 368;

/// Bullet group ids (ReC98 `bullet_group_t`). `_AIMED` puts 0° at the player.
pub mod group {
    pub const SINGLE: u8 = 0x00;
    pub const SINGLE_AIMED: u8 = 0x01;
    pub const FORCESINGLE_RANDOM_ANGLE: u8 = 0x1A;
    pub const RANDOM_ANGLE: u8 = 0x1B;
    pub const RANDOM_ANGLE_AND_SPEED: u8 = 0x1C;
    pub const RANDOM_CONSTRAINED_ANGLE_AIMED: u8 = 0x1D;
    pub const RING: u8 = 0x26;
    pub const RING_AIMED: u8 = 0x2C;
    pub const SPREAD: u8 = 0x2D;
    pub const SPREAD_AIMED: u8 = 0x2E;
    pub const STACK: u8 = 0x2F;
    pub const STACK_AIMED: u8 = 0x30;
    pub const FORCESINGLE: u8 = 0x40;
    pub const FORCESINGLE_AIMED: u8 = 0x41;
}

/// Bullet spawn parameters an enemy script builds up before firing.
#[derive(Default, Debug, Clone, Copy)]
pub struct BulletTemplate {
    pub spawn_type: u8,
    /// Origin relative to the firing enemy (subpixels).
    pub origin_x: i16,
    pub origin_y: i16,
    pub group: u8,
    pub angle: u8,
    pub speed: u8,
    pub patnum: u8,
    pub count: u8,
    /// Spread angle (for SPREAD) or stack speed delta (for STACK).
    pub delta: u8,
}

#[derive(Debug, Clone, Copy)]
pub struct Bullet {
    pub x: i32,
    pub y: i32,
    pub vx: i32,
    pub vy: i32,
    pub angle: u8,
    pub speed: i32,
    pub patnum: u8,
    pub active: bool,
}

#[derive(Default)]
pub struct BulletPool {
    pub bullets: Vec<Bullet>,
    rng: u32,
}

impl BulletPool {
    pub fn new() -> Self {
        Self { bullets: Vec::new(), rng: 0x2345_6789 }
    }

    pub fn active_count(&self) -> usize {
        self.bullets.iter().filter(|b| b.active).count()
    }

    fn rand8(&mut self) -> u8 {
        self.rng = self.rng.wrapping_mul(1103515245).wrapping_add(12345);
        (self.rng >> 16) as u8
    }

    fn add(&mut self, x: i32, y: i32, angle: u8, speed: i32, patnum: u8) {
        let (vx, vy) = vector2(angle, speed);
        let b = Bullet { x, y, vx, vy, angle, speed, patnum, active: true };
        match self.bullets.iter_mut().find(|s| !s.active) {
            Some(slot) => *slot = b,
            None => self.bullets.push(b),
        }
    }

    /// Spawn bullets from `t`, fired by an enemy at `(ex, ey)` (subpixels),
    /// aiming at `player` for `_AIMED` groups.
    pub fn spawn(&mut self, t: &BulletTemplate, ex: i32, ey: i32, player: (i32, i32)) {
        use group::*;
        let ox = ex + t.origin_x as i32;
        let oy = ey + t.origin_y as i32;
        let speed = t.speed as i32;
        let count = (t.count as i32).max(1); // guard ZUN's divide-by-zero bug
        let aimed = matches!(
            t.group,
            SINGLE_AIMED | RING_AIMED | SPREAD_AIMED | STACK_AIMED
                | FORCESINGLE_AIMED | RANDOM_CONSTRAINED_ANGLE_AIMED
        );
        let aim = if aimed { iatan2(player.1 - oy, player.0 - ox) } else { 0 };
        let base = t.angle.wrapping_add(aim);

        match t.group {
            RING | RING_AIMED => {
                for i in 0..count {
                    self.add(ox, oy, base.wrapping_add(((i * 0x100) / count) as u8), speed, t.patnum);
                }
            }
            SPREAD | SPREAD_AIMED => {
                // [count]-way arc centred on `base`, `delta` units between each.
                for i in 0..count {
                    let off = ((2 * i - (count - 1)) * t.delta as i32) / 2;
                    self.add(ox, oy, base.wrapping_add(off as u8), speed, t.patnum);
                }
            }
            STACK | STACK_AIMED => {
                for i in 0..count {
                    self.add(ox, oy, base, speed + i * t.delta as i32, t.patnum);
                }
            }
            RANDOM_ANGLE | RANDOM_ANGLE_AND_SPEED => {
                for _ in 0..count {
                    let a = self.rand8();
                    self.add(ox, oy, a, speed, t.patnum);
                }
            }
            FORCESINGLE_RANDOM_ANGLE => {
                let a = self.rand8();
                self.add(ox, oy, a, speed, t.patnum);
            }
            // SINGLE / SINGLE_AIMED / FORCESINGLE / FORCESINGLE_AIMED / unknown
            _ => self.add(ox, oy, base, speed, t.patnum),
        }
    }

    /// Advance every bullet one frame; cull those well off the playfield.
    pub fn update(&mut self) {
        let m = 16 * SUBPIXEL;
        for b in self.bullets.iter_mut() {
            if !b.active {
                continue;
            }
            b.x += b.vx;
            b.y += b.vy;
            if b.x < -m || b.x > PLAYFIELD_W * SUBPIXEL + m || b.y < -m || b.y > PLAYFIELD_H * SUBPIXEL + m {
                b.active = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_spawns_count_evenly() {
        let mut p = BulletPool::new();
        let t = BulletTemplate { group: group::RING, count: 8, speed: 32, ..Default::default() };
        p.spawn(&t, 0, 0, (0, 0));
        assert_eq!(p.active_count(), 8);
        // angles evenly spaced by 256/8 = 32
        let angles: Vec<u8> = p.bullets.iter().map(|b| b.angle).collect();
        assert_eq!(angles, [0, 32, 64, 96, 128, 160, 192, 224]);
    }

    #[test]
    fn aimed_single_points_at_player() {
        let mut p = BulletPool::new();
        let t = BulletTemplate { group: group::SINGLE_AIMED, count: 1, speed: 64, ..Default::default() };
        // player straight below the origin -> angle 64 (down)
        p.spawn(&t, 0, 0, (0, 1000));
        assert_eq!(p.bullets[0].angle, 64);
    }

    #[test]
    fn spread_is_centered() {
        let mut p = BulletPool::new();
        let t = BulletTemplate { group: group::SPREAD, count: 4, speed: 32, delta: 12, ..Default::default() };
        p.spawn(&t, 0, 0, (0, 0));
        let a: Vec<i32> = p.bullets.iter().map(|b| b.angle as i8 as i32).collect();
        assert_eq!(a, [-18, -6, 6, 18]); // centred 4-way, 12 apart
    }
}
