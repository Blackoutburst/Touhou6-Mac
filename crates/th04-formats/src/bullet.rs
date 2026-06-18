//! TH04 bullet system: spawn bullets from a [`BulletTemplate`] by group, and
//! move them each frame with the full TH04 motion model — straight flight, the
//! slow-bullet **decelerate ramp** (`BMF_DECELERATE`), and the **special
//! motions** (`BSM_*`). Faithfully follows ReC98 `th04/main/bullet/update.cpp`
//! (`bullet_update_special`, the `BMF_DECELERATE` branch) and `types.h`.
//!
//! Positions/velocities are subpixels (16/px); angles are 256-direction bytes
//! (0 = +x, 64 = +y).

use crate::math::{iatan2, vector2};

const SUBPIXEL: i32 = 16;
const PLAYFIELD_W: i32 = 384;
const PLAYFIELD_H: i32 = 368;
/// `BMF_DECELERATE_BASE_SPEED` = 4.5px.
const DECEL_BASE: i32 = 72;
/// `BMF_DECELERATE_THRESHOLD` = 4.0px: regular bullets slower than this ramp.
const DECEL_THRESHOLD: i32 = 64;
const DECEL_FRAMES: i32 = 32;

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

/// Special motion types (`bullet_special_motion_t`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Bsm {
    #[default]
    None,
    /// Decelerate to 0, then aim at the player and resume initial speed.
    DecelThenTurnAimed,
    /// Decelerate to 0, then `angle += turn_by` and resume initial speed.
    DecelThenTurn,
    /// `speed += speed_delta` every frame.
    Speedup,
    /// Decelerate to 0 while turning toward `target`, then resume at `target`.
    DecelToAngle,
    BounceLeftRight,
    BounceTopBottom,
    BounceLeftRightTopBottom,
    BounceLeftRightTop,
    /// `velocity.y += speed_delta` every two frames.
    Gravity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MoveFlag {
    Decelerate,
    Special,
    Regular,
}

/// Bullet spawn parameters an enemy/boss builds up before firing.
#[derive(Default, Debug, Clone, Copy)]
pub struct BulletTemplate {
    pub spawn_type: u8,
    /// Origin relative to the firing entity (subpixels).
    pub origin_x: i16,
    pub origin_y: i16,
    pub group: u8,
    pub angle: u8,
    pub speed: u8,
    pub patnum: u8,
    pub count: u8,
    /// Spread angle (for SPREAD) or stack speed delta (for STACK).
    pub delta: u8,
    /// Special motion applied to every spawned bullet.
    pub special: Bsm,
    /// `bullet_template_special_angle`: turn_by / target for the decel-turn motions.
    pub turn_arg: u8,
    /// `bullet_special.speed_delta` (SPEEDUP / GRAVITY), in subpixels.
    pub speed_delta: u8,
    /// `bullet_special.turns_max` for the turning/bouncing motions.
    pub turns_max: u8,
}

#[derive(Debug, Clone, Copy)]
pub struct Bullet {
    pub x: i32,
    pub y: i32,
    pub vx: i32,
    pub vy: i32,
    pub angle: u8,
    pub patnum: u8,
    pub active: bool,
    // --- motion state ---
    speed_cur: i32,
    speed_final: i32,
    move_flag: MoveFlag,
    special: Bsm,
    turns_done: u8,
    turns_max: u8,
    turn_arg: u8,
    speed_delta: i32,
    decel_time: i32,
    decel_delta: i32,
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

    /// Add one bullet, setting up its motion state from `t`.
    fn add(&mut self, t: &BulletTemplate, x: i32, y: i32, angle: u8, speed: i32, is_stack: bool) {
        // Decide the move flag (ReC98 spawn logic + update.cpp).
        let (move_flag, speed_cur, decel_time, decel_delta) = if t.special != Bsm::None {
            (MoveFlag::Special, speed, 0, 0)
        } else if !is_stack && speed < DECEL_THRESHOLD {
            (MoveFlag::Decelerate, DECEL_BASE, DECEL_FRAMES, DECEL_BASE - speed)
        } else {
            (MoveFlag::Regular, speed, 0, 0)
        };
        let (vx, vy) = vector2(angle, speed_cur);
        let b = Bullet {
            x,
            y,
            vx,
            vy,
            angle,
            patnum: t.patnum,
            active: true,
            speed_cur,
            speed_final: speed,
            move_flag,
            special: t.special,
            turns_done: 0,
            turns_max: t.turns_max,
            turn_arg: t.turn_arg,
            speed_delta: t.speed_delta as i32,
            decel_time,
            decel_delta,
        };
        match self.bullets.iter_mut().find(|s| !s.active) {
            Some(slot) => *slot = b,
            None => self.bullets.push(b),
        }
    }

    /// Spawn bullets from `t`, fired by an entity at `(ex, ey)` (subpixels),
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
                    self.add(t, ox, oy, base.wrapping_add(((i * 0x100) / count) as u8), speed, false);
                }
            }
            SPREAD | SPREAD_AIMED => {
                for i in 0..count {
                    let off = ((2 * i - (count - 1)) * t.delta as i32) / 2;
                    self.add(t, ox, oy, base.wrapping_add(off as u8), speed, false);
                }
            }
            STACK | STACK_AIMED => {
                for i in 0..count {
                    self.add(t, ox, oy, base, speed + i * t.delta as i32, true);
                }
            }
            RANDOM_ANGLE | RANDOM_ANGLE_AND_SPEED => {
                for _ in 0..count {
                    let a = self.rand8();
                    self.add(t, ox, oy, a, speed, false);
                }
            }
            FORCESINGLE_RANDOM_ANGLE => {
                let a = self.rand8();
                self.add(t, ox, oy, a, speed, false);
            }
            // SINGLE / SINGLE_AIMED / FORCESINGLE / FORCESINGLE_AIMED / unknown
            _ => self.add(t, ox, oy, base, speed, false),
        }
    }

    /// Advance every bullet one frame (`bullets_update`): apply its motion model,
    /// step, then cull off-screen bullets. `player` drives aimed turns; `frame`
    /// drives the gravity cadence.
    pub fn update(&mut self, player: (i32, i32), frame: u16) {
        let m = 16 * SUBPIXEL;
        for b in self.bullets.iter_mut() {
            if !b.active {
                continue;
            }
            match b.move_flag {
                MoveFlag::Special => b.update_special(player, frame),
                MoveFlag::Decelerate => {
                    b.decel_time -= 1;
                    b.speed_cur = b.speed_final + (b.decel_time * b.decel_delta) / DECEL_FRAMES;
                    if b.decel_time <= 0 {
                        b.speed_cur = b.speed_final;
                        b.move_flag = MoveFlag::Regular;
                    }
                    let (vx, vy) = vector2(b.angle, b.speed_cur);
                    b.vx = vx;
                    b.vy = vy;
                }
                MoveFlag::Regular => {}
            }
            b.x += b.vx;
            b.y += b.vy;
            if b.x < -m || b.x > PLAYFIELD_W * SUBPIXEL + m || b.y < -m || b.y > PLAYFIELD_H * SUBPIXEL + m {
                b.active = false;
            }
        }
    }
}

impl Bullet {
    fn set_velocity(&mut self) {
        let (vx, vy) = vector2(self.angle, self.speed_cur);
        self.vx = vx;
        self.vy = vy;
    }

    /// `bullet_turn_complete`: finish a decel-turn — restore speed, drop to
    /// regular once out of turns.
    fn turn_complete(&mut self) {
        self.speed_cur = self.speed_final;
        if self.turns_done >= self.turns_max {
            self.move_flag = MoveFlag::Regular;
        }
        self.set_velocity();
    }

    /// `bullet_update_special`: per-frame velocity for the special motions.
    fn update_special(&mut self, player: (i32, i32), frame: u16) {
        match self.special {
            Bsm::DecelThenTurnAimed => {
                if self.speed_cur != 0 {
                    self.set_velocity();
                    self.speed_cur -= 1;
                } else {
                    self.turns_done += 1;
                    self.angle = iatan2(player.1 - self.y, player.0 - self.x);
                    self.turn_complete();
                }
            }
            Bsm::DecelThenTurn => {
                if self.speed_cur != 0 {
                    self.set_velocity();
                    self.speed_cur -= 1;
                } else {
                    self.turns_done += 1;
                    self.angle = self.angle.wrapping_add(self.turn_arg);
                    self.turn_complete();
                }
            }
            Bsm::Speedup => {
                self.set_velocity();
                self.speed_cur += self.speed_delta;
            }
            Bsm::DecelToAngle => {
                if self.speed_cur != 0 {
                    self.set_velocity();
                    if self.speed_cur > 1 {
                        self.speed_cur -= 2;
                    } else {
                        self.speed_cur = 0;
                    }
                    if self.speed_cur < 2 * SUBPIXEL {
                        let d = (self.turn_arg as i8).wrapping_sub(self.angle as i8);
                        self.angle = self.angle.wrapping_add((d / 4) as u8);
                    }
                } else {
                    self.angle = self.turn_arg;
                    self.speed_cur = self.speed_final;
                    self.move_flag = MoveFlag::Regular;
                    self.set_velocity();
                }
            }
            Bsm::BounceLeftRight => self.bounce(true, false),
            Bsm::BounceTopBottom => self.bounce(false, true),
            Bsm::BounceLeftRightTopBottom => self.bounce(true, true),
            Bsm::BounceLeftRightTop => self.bounce_lrt(),
            Bsm::Gravity => {
                if frame % 2 != 0 {
                    self.vy += self.speed_delta;
                }
            }
            Bsm::None => {}
        }
    }

    fn turn_x(&mut self) {
        self.turns_done += 1;
        self.angle = 0x80u8.wrapping_sub(self.angle);
        if self.turns_done >= self.turns_max {
            self.move_flag = MoveFlag::Regular;
        }
        self.set_velocity();
    }
    fn turn_y(&mut self) {
        self.turns_done += 1;
        self.angle = (self.angle as i8).wrapping_neg() as u8;
        if self.turns_done >= self.turns_max {
            self.move_flag = MoveFlag::Regular;
        }
        self.set_velocity();
    }
    fn bounce(&mut self, lr: bool, tb: bool) {
        if lr && (self.x <= 0 || self.x >= PLAYFIELD_W * SUBPIXEL) {
            self.turn_x();
        }
        if tb && (self.y <= 0 || self.y >= PLAYFIELD_H * SUBPIXEL) {
            self.turn_y();
        }
    }
    fn bounce_lrt(&mut self) {
        if self.x <= 0 || self.x >= PLAYFIELD_W * SUBPIXEL {
            self.turn_x();
        }
        if self.y <= 0 {
            self.turn_y();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fast_ring(count: u8) -> BulletTemplate {
        // High speed avoids the decelerate ramp so spawn angles are testable.
        BulletTemplate { group: group::RING, count, speed: 64, ..Default::default() }
    }

    #[test]
    fn ring_spawns_count_evenly() {
        let mut p = BulletPool::new();
        p.spawn(&fast_ring(8), 0, 0, (0, 0));
        assert_eq!(p.active_count(), 8);
        let angles: Vec<u8> = p.bullets.iter().map(|b| b.angle).collect();
        assert_eq!(angles, [0, 32, 64, 96, 128, 160, 192, 224]);
    }

    #[test]
    fn aimed_single_points_at_player() {
        let mut p = BulletPool::new();
        let t = BulletTemplate { group: group::SINGLE_AIMED, count: 1, speed: 64, ..Default::default() };
        p.spawn(&t, 0, 0, (0, 1000));
        assert_eq!(p.bullets[0].angle, 64);
    }

    #[test]
    fn spread_is_centered() {
        let mut p = BulletPool::new();
        let t = BulletTemplate { group: group::SPREAD, count: 4, speed: 64, delta: 12, ..Default::default() };
        p.spawn(&t, 0, 0, (0, 0));
        let a: Vec<i32> = p.bullets.iter().map(|b| b.angle as i8 as i32).collect();
        assert_eq!(a, [-18, -6, 6, 18]);
    }

    #[test]
    fn slow_bullet_decelerates_from_base() {
        let mut p = BulletPool::new();
        // Speed 2px (32 subpx) < threshold → starts near 4.5px then ramps down.
        let t = BulletTemplate { group: group::SINGLE, count: 1, speed: 32, angle: 0, ..Default::default() };
        p.spawn(&t, 0, 0, (0, 0));
        let start_vx = p.bullets[0].vx;
        assert!(start_vx > 32, "should start faster than its final 2px speed");
        for _ in 0..40 {
            p.update((0, 0), 0);
        }
        // After the ramp, velocity settles to the final 2px speed.
        assert_eq!(p.bullets[0].vx, 32);
    }

    #[test]
    fn speedup_accelerates() {
        let mut p = BulletPool::new();
        let t = BulletTemplate {
            group: group::SINGLE,
            count: 1,
            speed: 64,
            angle: 0,
            special: Bsm::Speedup,
            speed_delta: 4,
            ..Default::default()
        };
        p.spawn(&t, 0, 0, (0, 0));
        let v0 = p.bullets[0].vx;
        p.update((0, 0), 0);
        p.update((0, 0), 0);
        assert!(p.bullets[0].vx > v0, "speedup should accelerate");
    }
}
