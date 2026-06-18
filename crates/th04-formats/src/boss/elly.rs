//! Stage 3 boss **Elly** — `@elly_update$qv` (th04_main.asm:24137) + pattern
//! subs `elly_1BC73/1BC3C/1BDB4/1BE43/1BD4B..1C251`. HP 6000.
//!
//! Phases: init → invincible approach → settle → **the attack** → defeat. The
//! attack (phase 3) is a 5-tier machine (`byte_25A24`): tiers advance at HP
//! 4700/3300/2100/700 (or after a per-tier pattern count), and each tier draws
//! its modes from a window of the ten patterns — tier 0 cycles modes {0,1};
//! tier 1 {0..3}; tier 2 {2..5}; tier 3 {4..7}; tier 4 {5..8}. Between patterns,
//! a 32-frame interlude moves Elly along her figure-8 orbit (`elly_1BC73`).
//!
//! Exact: the figure-8 movement, the per-tier mode selection, every pattern's
//! danmaku (angles/speeds/counts/cadence), and the shared 48-ring finale. The
//! one approximation is `byte_25A26` — the aim-animation counter that ends the
//! orbiting patterns, managed by the 290-line `elly_1B95C` sprite machine — which
//! is modelled here as a fixed ~64-frame countdown (invisible to gameplay).

use super::{Boss, BossEnv, SUBPIXEL};
use crate::bullet::{group, BulletTemplate};
use crate::math::{cos8, iatan2, sin8};

const PAT_KNIFE_YELLOW: u8 = 3;
const PAT_BALL_WHITE: u8 = 1;
const PAT_BALL_BLUE: u8 = 2;

// Per-tier mode selection (off_1C692): mode = (phase_state % MOD) + BASE, with a
// pattern-count limit that force-advances the tier.
const TIER_MOD: [i32; 5] = [2, 4, 4, 4, 4];
const TIER_BASE: [i32; 5] = [0, 0, 2, 4, 5];
const TIER_LIMIT: [i32; 5] = [8, 16, 24, 32, 40];
const TIER_HP: [i32; 4] = [4700, 3300, 2100, 700];
/// Approximated `byte_25A26` lifetime (frames the orbiting patterns fire for).
const AIM_FRAMES: i32 = 64;

impl Boss {
    pub(super) fn elly_update(&mut self, env: &mut BossEnv) {
        match self.phase {
            0 => {
                // loc_1C301: init. HP/thresholds set in the constructor.
                self.phase = 1;
                self.phase_frame = 0;
            }
            1 => self.elly_phase1_approach(),
            2 => self.elly_phase2_settle(env),
            3 => self.elly_phase3_attack(env),
            _ => self.defeat_frame += 1,
        }
    }

    /// Phase 1 (loc_1C32D): invincible side-stepping approach. The original ends
    /// it at a fixed global stage frame; we use a fixed ~3.5s local duration
    /// (`sb[5]` is a free approach timer; the sidestep flips every 65 frames).
    fn elly_phase1_approach(&mut self) {
        self.sb[5] += 1;
        if self.sb[5] % 65 == 0 {
            self.phase_state = (self.phase_state + 1) % 4;
        }
        self.vx = if matches!(self.phase_state, 0 | 3) { -SUBPIXEL } else { SUBPIXEL };
        self.x += self.vx;
        self.hittest_invincible();
        if self.sb[5] >= 210 {
            self.phase = 2;
            self.phase_frame = 0;
            self.vy = 8;
            self.sb[5] = 0;
        }
    }

    /// Phase 2 (loc_1C3D4): invincible — descend + servo to centre, then open
    /// the attack at tier 0.
    fn elly_phase2_settle(&mut self, env: &mut BossEnv) {
        self.y += self.vy;
        self.vx = if self.x < 192 * SUBPIXEL {
            2 * SUBPIXEL
        } else if self.x >= 193 * SUBPIXEL {
            -2 * SUBPIXEL
        } else {
            0
        };
        self.x += self.vx;
        self.phase_frame += 1;
        self.hittest_invincible();
        if self.phase_frame >= 32 {
            self.vx = 0;
            self.vy = 0;
            self.sb = [0; 8]; // [tier, word_25A3A, radius, aim_cd, aim_angle, fire_angle]
            self.x = 192 * SUBPIXEL;
            self.y = 96 * SUBPIXEL;
            self.phase_next(0, env.pool); // → phase 3, hp=6000, phase_end=0
            self.phase_state = 0;
        }
    }

    /// `elly_1BC73`: figure-8 orbit around (192, 96) at radius `sb[2]`, sweeping
    /// `angle` by the `word_25A3A` (`sb[1]`) schedule.
    fn elly_move(&mut self) {
        let w = self.sb[1];
        if w < 128 {
            self.sb[2] += 8;
            self.angle = 96;
        } else if w < 256 {
            self.angle = self.angle.wrapping_sub(1);
        } else if w < 384 {
            self.sb[2] -= 8;
        } else if w < 512 {
            self.sb[2] += 8;
            self.angle = 32;
        } else if w < 640 {
            self.angle = self.angle.wrapping_add(1);
        } else if w < 768 {
            self.sb[2] -= 8;
        } else {
            self.sb[2] += 8;
            self.angle = 96;
            self.sb[1] = 0;
        }
        let r = self.sb[2];
        self.x = 192 * SUBPIXEL + ((cos8(self.angle) * r) >> 8);
        self.y = 96 * SUBPIXEL + ((sin8(self.angle) * r) >> 8);
        self.sb[1] += 1;
    }

    /// `elly_1BC3C`: aim at the player; arm the orbiting-pattern countdown.
    fn elly_aim(&mut self, player: (i32, i32)) {
        self.sb[4] = iatan2(player.1 - self.y, player.0 - self.x) as i32;
        self.sb[3] = AIM_FRAMES;
    }

    fn elly_phase3_attack(&mut self, env: &mut BossEnv) {
        if self.sb[3] > 0 {
            self.sb[3] -= 1;
        }
        match self.mode {
            0 => self.elly_p_knife_spiral(env),
            1 => self.elly_p_pellet_storm(env, -0x40, 4, -2),
            2 => self.elly_p_aimed_ring(env, 8),
            3 => self.elly_p_cloud_fan(env, -0x40, 0x0B),
            4 => self.elly_p_dual_aimed(env),
            5 => self.elly_p_cloud_fan(env, 0x40, -0x0B),
            6 => self.elly_p_aimed_ring(env, 16),
            7 => self.elly_p_quad_random(env),
            8 => self.elly_p_blue_rings(env),
            _ => self.elly_interlude(env),
        }
        // loc_1C585: hit test (phase_frame++). phase_end_hp is 0 → only HP=0 ends.
        if self.hittest() {
            env.fx.spark(self.x, self.y);
            self.phase = 4;
            self.phase_frame = 0;
            self.begin_defeat(env.pool);
            return;
        }
        // loc_1C5B1: HP-threshold tier advance.
        let tier = self.sb[0] as usize;
        if tier < 4 && self.hp <= TIER_HP[tier] {
            self.sb[0] += 1;
            self.mode = -1;
            self.phase_frame = 0;
            for b in env.pool.bullets.iter_mut() {
                b.active = false;
            }
        }
    }

    /// mode -1 (loc_1C4A9): 32-frame figure-8 interlude, then pick the next mode
    /// from the current tier's window (or force-advance the tier on count).
    fn elly_interlude(&mut self, env: &mut BossEnv) {
        if self.phase_frame < 32 {
            self.elly_move();
            return;
        }
        self.phase_state += 1;
        let tier = self.sb[0] as usize;
        if self.phase_state as i32 >= TIER_LIMIT[tier] {
            if tier >= 4 {
                env.fx.spark(self.x, self.y);
                self.phase = 4;
                self.phase_frame = 0;
                self.begin_defeat(env.pool);
                return;
            }
            // loc_1C4EB: forced tier advance, HP reset to the tier boundary.
            self.sb[0] += 1;
            self.hp = 6000 - self.sb[0] * 1500;
            self.mode = -1;
        } else {
            self.mode = ((self.phase_state as i32 % TIER_MOD[tier]) + TIER_BASE[tier]) as i16;
        }
        self.phase_frame = 0;
    }

    /// `elly_1BE43`: the shared aimed 48-ring finale; ends the pattern.
    fn elly_finale_48ring(&mut self, env: &mut BossEnv) {
        let t = BulletTemplate {
            group: group::RING_AIMED,
            count: 48,
            speed: (2 * SUBPIXEL) as u8,
            patnum: PAT_BALL_BLUE,
            ..Default::default()
        };
        env.pool.spawn(&t, self.x, self.y, env.player);
        self.mode = -1;
        self.phase_frame = 0;
    }

    fn elly_orbiting_end(&mut self) {
        if self.phase_frame > 16 && self.sb[3] == 0 {
            self.mode = -1;
            self.phase_frame = 0;
        }
    }

    // --- mode 0 (elly_1BD4B): figure-8 + rotating aimed yellow knife spread ---
    fn elly_p_knife_spiral(&mut self, env: &mut BossEnv) {
        self.elly_move();
        let pf = self.phase_frame;
        if pf == 16 {
            self.elly_aim(env.player);
            self.sb[5] = -0x40;
        }
        if pf > 16 && env.frame % 16 == 0 {
            let t = BulletTemplate {
                group: group::SPREAD,
                count: 5,
                delta: 0x10,
                speed: (3 * SUBPIXEL) as u8,
                angle: self.sb[5] as u8,
                patnum: PAT_KNIFE_YELLOW,
                ..Default::default()
            };
            env.pool.spawn(&t, self.x, self.y, (0, 0));
            self.sb[5] = self.sb[5].wrapping_sub(0x10);
        }
        self.elly_orbiting_end();
    }

    // --- modes 1 (elly_1BE78): telegraph → pellet storm → 48-ring -----------
    // `aim_off` offsets the aim; the storm rotates +4 then (after frame 72) the
    // angle jumps +0x40 and rotates by `late_step`.
    fn elly_p_pellet_storm(&mut self, env: &mut BossEnv, aim_off: i32, _early: i32, late_step: i32) {
        let pf = self.phase_frame;
        if pf <= 32 {
            if pf == 1 {
                env.fx.gather(self.x, self.y);
            }
            if pf == 16 {
                env.fx.circle_shrink(self.x, self.y);
            }
            if pf == 32 {
                let aim = iatan2(env.player.1 - self.y, env.player.0 - self.x) as i32;
                self.sb[5] = aim + aim_off;
            }
            return;
        }
        let pellet = |angle: u8| BulletTemplate {
            group: group::SPREAD,
            count: 2,
            delta: 0x0C,
            speed: (4 * SUBPIXEL) as u8,
            angle,
            ..Default::default()
        };
        if pf < 72 {
            if pf % 2 == 0 {
                let t = pellet(self.sb[5] as u8);
                env.pool.spawn(&t, self.x, self.y, (0, 0));
                self.sb[5] += 4;
            }
        } else if pf == 72 {
            self.sb[5] += 0x40;
        } else if pf < 144 {
            if pf % 2 == 0 {
                let t = pellet(self.sb[5] as u8);
                env.pool.spawn(&t, self.x, self.y, (0, 0));
                self.sb[5] += late_step;
            }
        } else {
            self.elly_finale_48ring(env);
        }
    }

    // --- modes 2 / 6 (elly_1BF52 / elly_1C164): figure-8 + aimed pellet ring --
    fn elly_p_aimed_ring(&mut self, env: &mut BossEnv, count: u8) {
        self.elly_move();
        if count == 16 {
            self.elly_move(); // mode 6 orbits at double rate
        }
        let pf = self.phase_frame;
        if pf == 16 {
            self.elly_aim(env.player);
        }
        if pf > 16 && pf % 16 == 0 {
            let t = BulletTemplate {
                group: group::RING_AIMED,
                count,
                speed: (2 * SUBPIXEL) as u8,
                ..Default::default()
            };
            env.pool.spawn(&t, self.x, self.y, env.player);
        }
        self.elly_orbiting_end();
    }

    // --- modes 3 / 5 (elly_1BFAB / elly_1C0BF): telegraph → cloud fan → ring --
    fn elly_p_cloud_fan(&mut self, env: &mut BossEnv, aim_off: i32, step: i32) {
        let pf = self.phase_frame;
        if pf <= 32 {
            if pf == 1 {
                env.fx.gather(self.x, self.y);
            }
            if pf == 16 {
                env.fx.circle_shrink(self.x, self.y);
            }
            if pf == 32 {
                let aim = iatan2(env.player.1 - self.y, env.player.0 - self.x) as i32;
                self.sb[5] = aim + aim_off;
            }
            return;
        }
        if pf < 80 {
            if pf % 4 == 0 {
                let speed = ((2 * SUBPIXEL + 12) + pf / 8) as u8;
                let t = BulletTemplate {
                    group: group::SPREAD,
                    count: 4,
                    delta: 0x0C,
                    speed,
                    angle: self.sb[5] as u8,
                    patnum: PAT_BALL_WHITE,
                    special: crate::bullet::Bsm::None,
                    ..Default::default()
                };
                env.pool.spawn(&t, self.x, self.y, (0, 0));
                self.sb[5] += step;
            }
        } else if pf == 80 {
            self.sb[5] += 0x40;
        } else {
            self.elly_finale_48ring(env);
        }
    }

    // --- mode 4 (elly_1C044): figure-8 ×2 + mirrored aimed cloud pair ---------
    fn elly_p_dual_aimed(&mut self, env: &mut BossEnv) {
        self.elly_move();
        self.elly_move();
        let pf = self.phase_frame;
        if pf == 16 {
            self.elly_aim(env.player);
            self.sb[5] = 0x40;
        }
        if pf > 16 && env.frame % 8 == 0 {
            let one = |angle: u8| BulletTemplate {
                group: group::SINGLE_AIMED,
                count: 1,
                speed: (4 * SUBPIXEL) as u8,
                angle,
                patnum: PAT_BALL_BLUE,
                ..Default::default()
            };
            let a = self.sb[5];
            let t1 = one(a as u8);
            env.pool.spawn(&t1, self.x, self.y, env.player);
            let t2 = one((-(a as i8)) as u8);
            env.pool.spawn(&t2, self.x, self.y, env.player);
            self.sb[5] = (a - 3) as i8 as i32; // ZUN's neg/neg/-3
        }
        self.elly_orbiting_end();
    }

    // --- mode 7 (elly_1C1CF): telegraph → four random rings from corners ------
    fn elly_p_quad_random(&mut self, env: &mut BossEnv) {
        let pf = self.phase_frame;
        if pf <= 32 {
            if pf == 1 {
                env.fx.gather(self.x, self.y);
            }
            if pf == 32 {
                for (ox, oy) in [(-32, 0), (32, 0), (-32, -32), (-32, 32)] {
                    let t = BulletTemplate {
                        group: group::RING,
                        count: 16,
                        speed: (2 * SUBPIXEL) as u8,
                        angle: self.rng.byte(),
                        origin_x: (ox * SUBPIXEL) as i16,
                        origin_y: (oy * SUBPIXEL) as i16,
                        ..Default::default()
                    };
                    env.pool.spawn(&t, self.x, self.y, (0, 0));
                }
            }
            return;
        }
        if pf >= 80 {
            self.elly_finale_48ring(env);
        }
    }

    // --- mode 8 (elly_1C251): figure-8 ×2 + blue aimed 16-rings (+low-HP rng) --
    fn elly_p_blue_rings(&mut self, env: &mut BossEnv) {
        self.elly_move();
        self.elly_move();
        let pf = self.phase_frame;
        if pf == 16 {
            self.elly_aim(env.player);
        }
        if pf > 16 && pf % 16 == 0 {
            let t = BulletTemplate {
                group: group::RING_AIMED,
                count: 16,
                speed: (3 * SUBPIXEL + 8) as u8,
                patnum: PAT_BALL_BLUE,
                ..Default::default()
            };
            env.pool.spawn(&t, self.x, self.y, env.player);
            if self.hp <= 200 {
                let r = BulletTemplate {
                    group: group::RANDOM_ANGLE,
                    count: 2,
                    speed: (2 * SUBPIXEL) as u8,
                    ..Default::default()
                };
                env.pool.spawn(&r, self.x, self.y, (0, 0));
            }
        }
        self.elly_orbiting_end();
    }
}
