//! Stage 5 boss **Yuuka** — `@yuuka5_update$qv` (th04_main.asm:14335) + subs
//! `yuuka5_15ECE/15F97/160A5/161D7/162A3/1630D/16389/1653D`. HP 9000.
//!
//! Nineteen phases: intro → settle → [attack-A, glide, safety-circle] ×3 →
//! attack-B → glide → **Master Spark** → attack-B → glide → Master Spark →
//! final → defeat. HP descends 9000 → 7900 then −800 (attack-A) / −1100 or −1200
//! (safety-circle / spark) per phase. Between every attack pattern Yuuka glides
//! to a new spot via `yuuka5_15ECE`.
//!
//! Exact: the phase/HP machinery, the inter-pattern glide, the chasecross
//! sweep, spin-rings (+SPEEDUP cross bursts), the bouncing safety-circle crosses,
//! the accelerating aimed ring, the narrowing aimed spread, the final symmetric
//! spreads, and the Master Spark's rotating ring + random fill.
//!
//! Approximation: the Master Spark **thick laser** is point-based here — a dense
//! fast aimed column plus a `circle_grow` "safety-lane" telegraph (the original
//! draws a real thick beam + palette-tone safe zone). Documented deviation.

use super::{Boss, BossEnv, SUBPIXEL};
use crate::bullet::{group, Bsm, BulletTemplate};
use crate::math::iatan2;

const PAT_CROSS_YELLOW: u8 = 3;
const PAT_BALL_BLUE: u8 = 2;
const PAT_D_BLUE: u8 = 8;
const PAT_OUTLINED_BLUE: u8 = 1;
const RANK_NORMAL: u8 = 1;

impl Boss {
    pub(super) fn yuuka_update(&mut self, env: &mut BossEnv) {
        match self.phase {
            0 => {
                self.phase_frame += 1;
                self.hittest_invincible();
                if self.phase_frame > 128 {
                    self.phase = 1;
                    self.phase_frame = 0;
                }
            }
            1 => {
                self.phase_frame += 1;
                self.hittest_invincible();
                if self.phase_frame >= 64 {
                    self.phase = 2;
                    self.phase_frame = 0;
                    self.phase_state = 0;
                    self.mode = 0;
                    self.hp = 9000;
                    self.phase_end_hp = 7900;
                    self.y -= 16 * SUBPIXEL;
                }
            }
            2 | 5 | 8 => self.yuuka_attack(env, true),
            3 | 6 | 9 | 12 | 15 => {
                self.phase_frame += 1;
                if self.yuuka_move(true) {
                    self.phase += 1;
                    self.phase_frame = 0;
                    self.phase_state = 0;
                    self.mode = 0;
                }
            }
            4 | 7 | 10 => self.yuuka_safety_circle(env),
            11 | 14 => self.yuuka_attack(env, false),
            13 | 16 => self.yuuka_master_spark(env),
            17 => self.yuuka_final(env),
            _ => {
                self.phase_frame += 1;
                if self.phase_frame >= 32 {
                    env.fx.spark(self.x, self.y);
                    self.begin_defeat(env.pool);
                }
            }
        }
    }

    /// Bullet origin (`boss + (0, 16)`).
    fn yuuka_bo(&self) -> (i32, i32) {
        (self.x, self.y + 16 * SUBPIXEL)
    }

    /// `yuuka5_15ECE`: multi-stage glide. `center` = settle at (192,80), else a
    /// random spot. Returns true the frame it completes (and sets `mode = -2`).
    fn yuuka_move(&mut self, center: bool) -> bool {
        if self.sb[0] == 0 {
            self.sb[0] = 1;
            self.damage_pending = 0;
            self.y += 16 * SUBPIXEL;
        }
        match self.sb[0] {
            1 => {
                if self.phase_frame >= 32 {
                    self.phase_frame = 0;
                    self.sb[0] = 2;
                    let (tx, ty) = if center {
                        (192 * SUBPIXEL, 80 * SUBPIXEL)
                    } else {
                        (
                            self.rng.modulo(256 * SUBPIXEL) + 64 * SUBPIXEL,
                            self.rng.modulo(64 * SUBPIXEL) + 64 * SUBPIXEL,
                        )
                    };
                    self.vx = (tx - self.x) / 64;
                    self.vy = (ty - self.y) / 64;
                }
            }
            2 => {
                self.x += self.vx;
                self.y += self.vy;
                if self.phase_frame >= 64 {
                    self.phase_frame = 0;
                    self.sb[0] = 3;
                }
            }
            3 => {
                if self.phase_frame >= 8 {
                    self.phase_frame = 0;
                    self.sb[0] = 0;
                    self.mode = -2;
                    self.y -= 16 * SUBPIXEL;
                    return true;
                }
            }
            _ => {}
        }
        false
    }

    /// Attack phase (2/5/8 = A; 11/14 = B). `attack_a` selects the mode pair and
    /// the −800 HP step; B uses no HP step.
    fn yuuka_attack(&mut self, env: &mut BossEnv, attack_a: bool) {
        match self.mode {
            0 if attack_a => self.yuuka_chasecross(env),
            1 if attack_a => self.yuuka_spin_rings(env),
            0 => self.yuuka_aimed_ring(env),
            1 => self.yuuka_narrow_spread(env),
            -2 => {
                self.phase_frame = 0;
                self.phase_state += 1;
                self.mode = self.phase_state % 2;
            }
            _ => {
                self.yuuka_move(false); // 15ECE(0) between patterns
            }
        }
        if self.sb[0] == 0 {
            let end = self.phase_state >= 4 || self.hittest();
            if end {
                for b in env.pool.bullets.iter_mut() {
                    b.active = false;
                }
                self.phase += 1;
                self.hp = self.phase_end_hp;
                if attack_a {
                    self.phase_end_hp -= 800;
                }
                self.phase_frame = 0;
                self.phase_state = 0;
                self.mode = 0;
            }
        } else {
            self.phase_frame += 1;
        }
    }

    /// Phases 4/7/10 (`yuuka5_161D7`): bouncing safety-circle crosses.
    fn yuuka_safety_circle(&mut self, env: &mut BossEnv) {
        let (bx, by) = self.yuuka_bo();
        match self.phase_frame {
            1 | 3 | 5 => env.fx.gather(bx, by),
            0x11 => env.fx.circle_shrink(bx, by),
            _ => {}
        }
        if self.phase_frame >= 32 && self.phase_frame % 16 == 0 {
            let t = BulletTemplate {
                spawn_type: 2,
                group: group::SPREAD,
                count: 7,
                delta: 8,
                speed: (2 * SUBPIXEL) as u8,
                angle: self.rng.byte(),
                patnum: PAT_CROSS_YELLOW,
                special: Bsm::BounceLeftRightTop,
                turns_max: 1,
                ..Default::default()
            };
            env.pool.spawn(&t, bx, by, (0, 0));
        }
        let end = self.phase_frame >= 500 || self.hittest();
        if end {
            for b in env.pool.bullets.iter_mut() {
                b.active = false;
            }
            self.phase += 1;
            self.hp = self.phase_end_hp;
            self.phase_end_hp -= if self.phase < 0x0A { 1100 } else { 1200 };
            self.phase_frame = 0;
            self.phase_state = 0;
            self.mode = 0;
        }
    }

    /// Phases 13/16 (`yuuka5_16389`): Master Spark.
    fn yuuka_master_spark(&mut self, env: &mut BossEnv) {
        let (bx, by) = self.yuuka_bo();
        // Charge telegraph.
        if matches!(self.phase_frame, 0x28 | 0x2C | 0x30 | 0x38 | 0x40 | 0x48 | 0x50) {
            env.fx.gather(bx, by);
        }
        if matches!(self.phase_frame, 0x38 | 0x48 | 0x50) {
            env.fx.circle_shrink(bx, by);
        }
        if self.phase_frame == 0x60 {
            env.fx.circle_grow(bx, by); // thick-laser fire telegraph
        }
        // The thick beam (approximated): a dense fast aimed column once firing.
        if self.phase_frame >= 96 && self.phase_frame < 240 && self.phase_frame % 2 == 0 {
            let aim = iatan2(env.player.1 - by, env.player.0 - bx);
            let beam = BulletTemplate {
                spawn_type: 2,
                group: group::SPREAD,
                count: 5,
                delta: 3,
                speed: (8 * SUBPIXEL) as u8,
                angle: aim,
                patnum: PAT_CROSS_YELLOW,
                ..Default::default()
            };
            env.pool.spawn(&beam, bx, by, (0, 0));
            env.fx.circle_grow(bx, by);
        }
        // Background rotating ring (frame >= 128, every 32).
        if self.phase_frame >= 128 && self.phase_frame % 32 == 0 {
            let t = BulletTemplate {
                spawn_type: 2,
                group: group::RING,
                count: 32,
                speed: (4 * SUBPIXEL + 8) as u8,
                angle: self.angle,
                patnum: PAT_D_BLUE,
                ..Default::default()
            };
            env.pool.spawn(&t, bx, by, (0, 0));
            self.angle = self.angle.wrapping_add(2);
        }
        // Random fill (frame >= 192, every other frame).
        if self.phase_frame >= 192 && self.phase_frame % 2 != 0 {
            let t = BulletTemplate {
                spawn_type: 2,
                group: group::RANDOM_ANGLE,
                count: 2,
                speed: (2 * SUBPIXEL) as u8,
                patnum: PAT_OUTLINED_BLUE,
                ..Default::default()
            };
            env.pool.spawn(&t, bx, by, (0, 0));
        }
        self.phase_frame += 1;
        self.hittest_invincible();
        if self.phase_frame >= 288 {
            for b in env.pool.bullets.iter_mut() {
                b.active = false;
            }
            self.phase += 1;
            self.hp = self.phase_end_hp;
            if self.phase == 0x11 {
                self.phase_end_hp = 0;
            } else {
                self.phase_end_hp -= 1200;
            }
            self.phase_frame = 0;
            self.phase_state = 0;
            self.mode = 0;
        }
    }

    /// Phase 17 (`yuuka5_1653D`): the final symmetric spreads; dies at HP 0 or a
    /// 1000-frame timeout.
    fn yuuka_final(&mut self, env: &mut BossEnv) {
        let (bx, by) = self.yuuka_bo();
        if self.phase_frame == 48 {
            env.fx.circle_shrink(bx, by);
            self.angle = 16;
            self.sb[6] = 0x10; // spread_angle
        }
        if self.phase_frame >= 64 && self.phase_frame % 8 == 0 {
            // Twin mirrored cloud spreads.
            let cloud = |angle: u8, ox: i32| BulletTemplate {
                spawn_type: 4,
                group: group::SPREAD,
                count: 5,
                delta: 1,
                speed: (2 * SUBPIXEL + 8) as u8,
                angle,
                origin_x: ox as i16,
                patnum: PAT_BALL_BLUE,
                ..Default::default()
            };
            let a = self.angle;
            let c1 = cloud(a, 32 * SUBPIXEL);
            env.pool.spawn(&c1, bx, by, (0, 0));
            let c2 = cloud(0x80u8.wrapping_sub(a), -32 * SUBPIXEL);
            env.pool.spawn(&c2, bx, by, (0, 0));
            self.angle = self.angle.wrapping_sub(16);
            // Twin mirrored pellet spreads.
            let sa = self.sb[6] as u8;
            let pel = |angle: u8, ox: i32| BulletTemplate {
                spawn_type: 1,
                group: group::SPREAD,
                count: 3,
                delta: 1,
                speed: (SUBPIXEL + 8) as u8,
                angle,
                origin_x: ox as i16,
                ..Default::default()
            };
            let p1 = pel(sa, -32 * SUBPIXEL);
            env.pool.spawn(&p1, bx, by, (0, 0));
            let p2 = pel(0x80u8.wrapping_sub(sa), 32 * SUBPIXEL);
            env.pool.spawn(&p2, bx, by, (0, 0));
            self.sb[6] = (self.sb[6] + 9) & 0xFF;
        }
        let dead = self.hittest();
        if dead || self.phase_frame >= 1000 {
            env.fx.spark(self.x, self.y);
            self.phase_state = if self.phase_frame < 1000 { 1 } else { 0 };
            self.phase = 18;
            self.phase_frame = 0;
        }
    }

    // === Attack-A modes ===================================================

    /// `yuuka5_15F97`: the 11-way cross spread sweeping angle + origin.
    fn yuuka_chasecross(&mut self, env: &mut BossEnv) {
        let pf = self.phase_frame;
        if pf == 1 {
            self.sb[5] = self.x - 32 * SUBPIXEL; // word_25662 (origin x)
            self.angle = 0; // BT_angle
        } else {
            // Frames 31,47,... set the base angle and step the origin ±64px.
            let steps: [(i32, u8, i32); 8] = [
                (15, 0, 0),
                (31, 0x80, 64),
                (47, 0x10, -64),
                (63, 0x70, 64),
                (79, 0x20, -64),
                (95, 0x60, 64),
                (111, 0x30, -64),
                (127, 0x50, 64),
            ];
            for (f, ang, dx) in steps {
                if pf == f {
                    self.angle = ang;
                    self.sb[5] += dx * SUBPIXEL;
                }
            }
        }
        if pf == 140 {
            self.mode = -1;
            self.phase_frame = 0;
        }
        if pf % 16 == 15 {
            let t = BulletTemplate {
                spawn_type: 2,
                group: group::SPREAD,
                count: 11,
                delta: 5,
                speed: (2 * SUBPIXEL + 8) as u8,
                angle: self.angle,
                patnum: PAT_CROSS_YELLOW,
                ..Default::default()
            };
            env.pool.spawn(&t, self.sb[5], self.y, (0, 0));
        }
    }

    /// `yuuka5_160A5`: an accelerating rotating ring + two SPEEDUP cross bursts.
    fn yuuka_spin_rings(&mut self, env: &mut BossEnv) {
        let (bx, by) = self.yuuka_bo();
        let pf = self.phase_frame;
        let ring_count = RANK_NORMAL + 1;
        if pf == 1 {
            self.angle = 0;
            self.sb[3] = 1; // byte_25664 (accel)
            self.sb[4] = 0; // byte_25665 (accumulator)
            return;
        }
        if pf == 128 || pf == 256 {
            let t = BulletTemplate {
                spawn_type: 2,
                group: group::RING,
                count: 32,
                speed: SUBPIXEL as u8,
                angle: if pf == 128 { 0x80 } else { 0 },
                patnum: PAT_CROSS_YELLOW,
                special: Bsm::Speedup,
                speed_delta: 1,
                ..Default::default()
            };
            env.pool.spawn(&t, bx, by, (0, 0));
            if pf == 128 {
                self.sb[3] = 1;
                self.sb[4] = 0;
            }
            return;
        }
        if pf == 288 {
            self.mode = -1;
            self.phase_frame = 0;
            return;
        }
        // Rotating ring, gated by the accelerating accumulator.
        if pf < 256 && self.sb[4] >= 0x10 {
            let step: i8 = if pf < 128 { 7 } else { -7 };
            self.angle = self.angle.wrapping_add(step as u8);
            let t = BulletTemplate {
                spawn_type: 4,
                group: group::RING,
                count: ring_count,
                speed: (4 * SUBPIXEL) as u8,
                angle: self.angle,
                patnum: PAT_D_BLUE,
                ..Default::default()
            };
            env.pool.spawn(&t, bx, by, (0, 0));
            self.sb[3] += 1;
            self.sb[4] -= 0x10;
        }
        self.sb[4] += self.sb[3];
    }

    // === Attack-B modes ===================================================

    /// `yuuka5_162A3`: an aimed cross ring that grows + speeds up.
    fn yuuka_aimed_ring(&mut self, env: &mut BossEnv) {
        let (bx, by) = self.yuuka_bo();
        if self.phase_frame == 1 {
            self.sb[3] = 8; // ring count
        }
        if self.phase_frame == 170 {
            self.mode = -1;
            self.phase_frame = 0;
        }
        if self.phase_frame % 16 == 15 {
            let t = BulletTemplate {
                spawn_type: 2,
                group: group::RING_AIMED,
                count: self.sb[3] as u8,
                speed: SUBPIXEL as u8,
                patnum: PAT_CROSS_YELLOW,
                special: Bsm::Speedup,
                speed_delta: 1,
                ..Default::default()
            };
            env.pool.spawn(&t, bx, by, env.player);
            self.sb[3] += 3;
        }
    }

    /// `yuuka5_1630D`: an aimed cloud spread whose arc narrows each volley.
    fn yuuka_narrow_spread(&mut self, env: &mut BossEnv) {
        let (bx, by) = self.yuuka_bo();
        if self.phase_frame == 1 {
            self.angle = iatan2(env.player.1 - by, env.player.0 - bx);
            self.sb[3] = 0x42; // spread angle
        }
        if self.phase_frame == 128 {
            self.mode = -1;
            self.phase_frame = 0;
        }
        if self.phase_frame % 8 == 7 {
            self.sb[3] -= 4;
            let t = BulletTemplate {
                spawn_type: 4,
                group: group::SPREAD,
                count: 5,
                delta: self.sb[3].max(1) as u8,
                speed: (5 * SUBPIXEL) as u8,
                angle: self.angle,
                patnum: PAT_BALL_BLUE,
                ..Default::default()
            };
            env.pool.spawn(&t, bx, by, (0, 0));
        }
    }
}
