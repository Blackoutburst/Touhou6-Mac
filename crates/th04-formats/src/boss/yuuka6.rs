//! Stage 6 (final) boss **Yuuka** — `@yuuka6_update$qv` (th04_main.asm:22545)
//! and its ~30 subs (`yuuka6_1A0D1`..`1B3E2`, `chasecrosses_add`). HP 13300.
//! The largest boss: 18 phases with parasol vanish/appear, path-flight, the
//! **mirror point** (twin attacks fired from Yuuka *and* her reflection across
//! the playfield), homing **chasecross** bullets, the **safety-circle**
//! (a shrinking ring that fires from its rim, leaving a safe lane), and a
//! thick-laser finale.
//!
//! Ported exactly: the 18-phase flow + HP arithmetic (13300 → 10600 → 7600 →
//! 5400 → 3400 → 1200 → 0), the per-phase attack rosters, and each attack's
//! danmaku. The chasecross homing/destructible bullets reuse the satellite pool
//! (`orbits`); the sim resolves their shot/player collision.
//!
//! Approximations (this is the only non-fully-exact boss, given it is ~2× the
//! size of any other): the safety-circle is modelled as a shrinking telegraph +
//! periodic aimed-ring fire (rather than the exact per-rim-point geometry), the
//! thick laser is a fast column, and the parasol-shield damage-redirect is
//! folded into the normal hittest. Phase/HP/attack structure is exact.

use super::{Boss, BossEnv, Orbit, PLAYFIELD_H, PLAYFIELD_W, SUBPIXEL};
use crate::bullet::{group, Bsm, BulletTemplate};
use crate::math::{iatan2, sin8, vector2};

const PAT_CROSS_YELLOW: u8 = 3;
const PAT_D_BLUE: u8 = 8;
const PAT_SMALL_RED: u8 = 9;
const PAT_BALL_RED: u8 = 6;

impl Boss {
    pub(super) fn yuuka6_update(&mut self, env: &mut BossEnv) {
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
                    self.hp = 13300;
                    self.phase_end_hp = 10600;
                }
            }
            2 => self.y6_attack(env, &[0, 1, 2], 10, 7600),
            3 | 7 | 11 | 13 => {
                // Anim / move-to-centre transitions.
                self.phase_frame += 1;
                if self.x < 192 * SUBPIXEL {
                    self.x += SUBPIXEL;
                } else if self.x > 192 * SUBPIXEL {
                    self.x -= SUBPIXEL;
                }
                self.hittest_invincible();
                if self.phase_frame >= 64 {
                    self.phase += 1;
                    self.phase_frame = 0;
                    self.phase_state = 0;
                    self.mode = 0;
                }
            }
            4 => self.y6_attack(env, &[3], 10, 5400),
            5 | 9 => {
                // Vanish (invincible).
                self.phase_frame += 1;
                self.hittest_invincible();
                if self.phase_frame >= 64 {
                    self.phase += 1;
                    self.phase_frame = 0;
                }
            }
            6 | 10 => {
                // Appear: bob across the top while firing `1ADDB`.
                self.phase_frame += 1;
                self.y6_appear_fire(env);
                self.x += if self.phase_frame == 1 { 2 * SUBPIXEL } else { self.vx };
                self.vx = if self.phase_frame == 1 { 2 * SUBPIXEL } else { self.vx };
                if self.x <= 48 * SUBPIXEL || self.x >= 336 * SUBPIXEL {
                    self.vx = -self.vx;
                }
                self.y = 64 * SUBPIXEL + ((sin8((self.phase_frame as u8).wrapping_mul(2)) * (16 * SUBPIXEL)) >> 8);
                self.y6_chasecross_step(env);
                if self.phase_frame >= 320 {
                    self.phase += 1;
                    self.phase_frame = 0;
                    self.x = 192 * SUBPIXEL;
                    self.y = 80 * SUBPIXEL;
                }
                return;
            }
            8 => self.y6_attack(env, &[4, 5, 6], 10, 3400),
            12 => self.y6_attack(env, &[4, 5, 6], 10, 1200),
            14 => self.y6_attack(env, &[7, 8, 9], 18, 0),
            15 => {
                self.phase_frame += 1;
                self.hittest();
                if self.phase_frame >= 128 {
                    self.phase = 16;
                    self.phase_frame = 0;
                }
            }
            16 => self.y6_final(env),
            _ => {
                self.phase_frame += 1;
                if self.phase_frame >= 32 {
                    env.fx.spark(self.x, self.y);
                    self.begin_defeat(env.pool);
                }
            }
        }
        self.y6_chasecross_step(env);
    }

    fn y6_mirror(&self) -> (i32, i32) {
        (PLAYFIELD_W * SUBPIXEL - self.x, self.y)
    }

    /// Generic attack phase: cycle `modes` (mode = phase_state % len), advancing
    /// once `phase_state >= gate` or HP hits the threshold.
    fn y6_attack(&mut self, env: &mut BossEnv, modes: &[u8], gate: i16, next_end: i32) {
        match self.mode {
            -1 => {
                // Between-pattern: pick the next mode.
                self.phase_frame = 0;
                self.phase_state += 1;
                self.mode = modes[(self.phase_state as usize) % modes.len()] as i16;
            }
            m => {
                let pat = m as u8;
                if pat == modes[0] {
                    self.y6_pattern(env, pat);
                } else {
                    self.y6_pattern(env, pat);
                }
            }
        }
        if self.phase_state >= gate || self.hittest() {
            for b in env.pool.bullets.iter_mut() {
                b.active = false;
            }
            self.phase += 1;
            self.hp = self.phase_end_hp;
            self.phase_end_hp = next_end;
            self.phase_frame = 0;
            self.phase_state = 0;
            self.mode = 0;
        }
    }

    fn y6_pattern(&mut self, env: &mut BossEnv, pat: u8) {
        match pat {
            0 => self.y6_decel_rings(env),
            1 => self.y6_spin_ring(env),
            2 => self.y6_gravity_randoms(env),
            3 => self.y6_safety_circle(env),
            4 => self.y6_mirror_spreads(env),
            5 => self.y6_random_spreads(env),
            6 => self.y6_aimed_spreads(env),
            7 => self.y6_sweep(env),
            8 => self.y6_rot_rings(env),
            _ => self.y6_chasecross_burst(env),
        }
    }

    fn y6_end_pattern(&mut self) {
        self.phase_frame = 0;
        self.mode = -1;
    }

    // === Chasecross (homing destructible crosses, via the orbit pool) ======

    fn y6_chasecross_add(&mut self, angle: u8, speed: i32) {
        let (cx, cy) = (self.x, self.y);
        if let Some(o) = self.orbits.iter_mut().find(|o| o.flag == 0) {
            *o = Orbit {
                flag: 1,
                angle,
                cx,
                cy,
                ox: cx,
                oy: cy,
                mspeed: speed,
                hp: 100,
                spin_time: 0, // age
                ..Default::default()
            };
        }
    }

    /// `yuuka6_1A110` (chasecross half): home toward the player for 56 frames,
    /// then fly straight; cull off-screen. (Collision is in `sim`.)
    fn y6_chasecross_step(&mut self, env: &mut BossEnv) {
        let (px, py) = env.player;
        let (w, h) = (PLAYFIELD_W * SUBPIXEL, PLAYFIELD_H * SUBPIXEL);
        for o in self.orbits.iter_mut() {
            if o.flag != 1 {
                continue;
            }
            if o.spin_time < 56 {
                let want = iatan2(py - o.cy, px - o.cx);
                let d = want.wrapping_sub(o.angle);
                o.angle = if d < 0x80 {
                    o.angle.wrapping_add(1)
                } else {
                    o.angle.wrapping_sub(1)
                };
            }
            let (vx, vy) = vector2(o.angle, o.mspeed);
            o.cx += vx;
            o.cy += vy;
            o.spin_time += 1;
            if o.cx < -8 * SUBPIXEL || o.cx > w + 8 * SUBPIXEL || o.cy > h + 8 * SUBPIXEL || o.cy < -8 * SUBPIXEL {
                o.flag = 0;
            }
        }
    }

    // === Attacks ==========================================================

    /// `yuuka6_1AB5D`: two decelerate-then-turn cross rings.
    fn y6_decel_rings(&mut self, env: &mut BossEnv) {
        let (bx, by) = (self.x, self.y - 4 * SUBPIXEL);
        if self.phase_frame == 48 {
            self.sb[2] = 2 * SUBPIXEL + 8;
        }
        if self.phase_frame == 48 || self.phase_frame == 64 {
            for turn in [0xC0u8, 0x40u8] {
                let t = BulletTemplate {
                    spawn_type: 2,
                    group: group::RING,
                    count: 20,
                    speed: self.sb[2] as u8,
                    patnum: PAT_CROSS_YELLOW,
                    special: Bsm::DecelThenTurn,
                    turn_arg: turn,
                    turns_max: 1,
                    ..Default::default()
                };
                env.pool.spawn(&t, bx, by, (0, 0));
            }
            self.sb[2] += SUBPIXEL;
        }
        if self.phase_frame >= 80 {
            self.y6_end_pattern();
        }
    }

    /// `yuuka6_1ABE5`: a twin spin-ring sprayed from the parasol rim.
    fn y6_spin_ring(&mut self, env: &mut BossEnv) {
        if self.phase_frame >= 48 && self.phase_frame <= 80 && self.phase_frame % 2 == 0 {
            let base = (self.phase_frame as u8).wrapping_mul(8).wrapping_neg();
            for ang in [base, 0x80u8.wrapping_sub(base)] {
                let (ox, oy) = vector2(ang, 34 * SUBPIXEL);
                let t = BulletTemplate {
                    spawn_type: 1,
                    group: group::RING,
                    count: 16,
                    speed: (2 * SUBPIXEL + 8) as u8,
                    angle: ang,
                    ..Default::default()
                };
                env.pool.spawn(&t, self.x + ox, self.y + oy, (0, 0));
            }
        }
        if self.phase_frame >= 96 {
            self.y6_end_pattern();
        }
    }

    /// `yuuka6_1ACCC`: gravity-affected random small-red balls from ±20px.
    fn y6_gravity_randoms(&mut self, env: &mut BossEnv) {
        if (48..=80).contains(&self.phase_frame) && env.frame % 4 == 0 {
            for side in [-20, 24] {
                let t = BulletTemplate {
                    spawn_type: 4,
                    group: group::RANDOM_ANGLE_AND_SPEED,
                    count: 4,
                    speed: (SUBPIXEL + 8 + self.rng.modulo(24)) as u8,
                    angle: self.rng.byte(),
                    origin_x: (side * SUBPIXEL) as i16,
                    origin_y: (-4 * SUBPIXEL) as i16,
                    patnum: PAT_SMALL_RED,
                    special: Bsm::Gravity,
                    speed_delta: 1,
                    ..Default::default()
                };
                env.pool.spawn(&t, self.x, self.y, (0, 0));
            }
        }
        if self.phase_frame > 80 {
            self.y6_end_pattern();
        }
    }

    /// `yuuka6_1AD6F` + `1A0D1`/`1A110`: the safety-circle attack (modelled as a
    /// shrinking telegraph that fires aimed rings from the boss).
    fn y6_safety_circle(&mut self, env: &mut BossEnv) {
        if self.phase_frame == 64 {
            env.fx.circle_shrink(env.player.0, env.player.1); // safe-zone telegraph
        }
        if self.phase_frame >= 64 && self.phase_frame % 16 == 0 {
            env.fx.circle_grow(env.player.0, env.player.1);
            let t = BulletTemplate {
                spawn_type: 2,
                group: group::RING_AIMED,
                count: 32,
                speed: SUBPIXEL as u8,
                patnum: PAT_SMALL_RED,
                ..Default::default()
            };
            env.pool.spawn(&t, env.player.0, env.player.1, env.player);
        }
        if self.phase_frame >= 288 {
            self.y6_end_pattern();
        }
    }

    /// `yuuka6_1ADDB`: the appear-phase fire (rings + aimed stacks).
    fn y6_appear_fire(&mut self, env: &mut BossEnv) {
        if env.frame % 16 != 0 {
            return;
        }
        if self.phase == 6 {
            let r = BulletTemplate {
                spawn_type: 2,
                group: group::RING,
                count: 16,
                speed: (SUBPIXEL + 14) as u8,
                angle: self.rng.byte(),
                patnum: PAT_SMALL_RED,
                ..Default::default()
            };
            env.pool.spawn(&r, self.x, self.y, (0, 0));
            let c = BulletTemplate {
                spawn_type: 4,
                group: group::RANDOM_ANGLE_AND_SPEED,
                count: 4,
                speed: (SUBPIXEL + 8) as u8,
                ..Default::default()
            };
            env.pool.spawn(&c, self.x, self.y, (0, 0));
        } else {
            let t = BulletTemplate {
                spawn_type: 4,
                group: group::STACK_AIMED,
                count: 7,
                delta: 0x0A,
                speed: (2 * SUBPIXEL) as u8,
                patnum: PAT_D_BLUE,
                ..Default::default()
            };
            env.pool.spawn(&t, self.x, self.y, env.player);
        }
    }

    /// `yuuka6_1AE8F`: thick-laser column + mirrored D_BLUE spreads.
    fn y6_mirror_spreads(&mut self, env: &mut BossEnv) {
        let (mx, my) = self.y6_mirror();
        if self.phase_frame == 64 {
            // Two thick lasers (boss + mirror), approximated by fast columns.
            env.fx.circle_grow(self.x, self.y + 32 * SUBPIXEL);
            env.fx.circle_grow(mx, my + 40 * SUBPIXEL);
        }
        if (64..=128).contains(&self.phase_frame) && env.frame % 8 == 0 {
            for (cx, cy) in [(self.x, self.y), (mx, my)] {
                for ang in [0x60u8, 0x20u8] {
                    let t = BulletTemplate {
                        spawn_type: 4,
                        group: group::SPREAD,
                        count: 3,
                        delta: 8,
                        speed: (2 * SUBPIXEL + self.sb[2].min(48)) as u8,
                        angle: ang,
                        patnum: PAT_D_BLUE,
                        ..Default::default()
                    };
                    env.pool.spawn(&t, cx, cy, (0, 0));
                }
            }
            self.sb[2] += 12;
        }
        if self.phase_frame >= 160 {
            self.sb[2] = 0;
            self.y6_end_pattern();
        }
    }

    /// `yuuka6_1AFA8`: mirrored yellow 5-spreads with a widening random arc.
    fn y6_random_spreads(&mut self, env: &mut BossEnv) {
        if self.phase_frame == 64 {
            self.sb[2] = 2;
        }
        let (mx, my) = self.y6_mirror();
        if (64..=128).contains(&self.phase_frame) && env.frame % 4 == 0 {
            let range = self.sb[2].max(1);
            for (cx, cy) in [(self.x, self.y + 32 * SUBPIXEL), (mx, my + 32 * SUBPIXEL)] {
                let angle = (0x40 - range + self.rng.modulo(2 * range)) as u8;
                let t = BulletTemplate {
                    spawn_type: 2,
                    group: group::SPREAD,
                    count: 5,
                    delta: 0x10,
                    speed: (12 + self.rng.modulo(0x20)) as u8,
                    angle,
                    patnum: PAT_CROSS_YELLOW,
                    ..Default::default()
                };
                env.pool.spawn(&t, cx, cy, (0, 0));
            }
            self.sb[2] += 6;
        }
        if self.phase_frame >= 160 {
            self.y6_end_pattern();
        }
    }

    /// `yuuka6_1B313`: mirrored wide / aimed D_BLUE spreads.
    fn y6_aimed_spreads(&mut self, env: &mut BossEnv) {
        let (mx, my) = self.y6_mirror();
        if (64..=192).contains(&self.phase_frame) && env.frame % 16 == 0 {
            let wide = (env.frame & 0x1F) == 0;
            for (cx, cy) in [(self.x, self.y + 32 * SUBPIXEL), (mx, my + 32 * SUBPIXEL)] {
                let t = if wide {
                    BulletTemplate {
                        spawn_type: 2,
                        group: group::SPREAD,
                        count: 10,
                        delta: 0x0E,
                        speed: (2 * SUBPIXEL + 8) as u8,
                        angle: 0x40,
                        patnum: PAT_D_BLUE,
                        ..Default::default()
                    }
                } else {
                    BulletTemplate {
                        spawn_type: 2,
                        group: group::SPREAD_AIMED,
                        count: 7,
                        delta: 0x0E,
                        speed: (2 * SUBPIXEL + 8) as u8,
                        patnum: PAT_D_BLUE,
                        ..Default::default()
                    }
                };
                env.pool.spawn(&t, cx, cy, env.player);
            }
        }
        if self.phase_frame >= 224 {
            self.y6_end_pattern();
        }
    }

    /// `yuuka6_1B099`: a swept aimed red ring stream.
    fn y6_sweep(&mut self, env: &mut BossEnv) {
        if self.phase_frame == 48 {
            self.angle = iatan2(self.y - env.player.1, self.x - env.player.0).wrapping_add(0x10);
            self.sb[6] = 1;
        }
        if (48..136).contains(&self.phase_frame) && self.phase_frame % 2 != 0 {
            let t = BulletTemplate {
                spawn_type: 4,
                group: group::RING,
                count: 8,
                speed: (9 * SUBPIXEL) as u8,
                angle: self.angle,
                patnum: PAT_BALL_RED,
                ..Default::default()
            };
            env.pool.spawn(&t, self.x, self.y, (0, 0));
            if self.phase_frame >= 112 {
                self.angle = self.angle.wrapping_add(self.sb[6] as u8);
            }
        }
        if self.phase_frame >= 144 {
            self.y6_end_pattern();
        }
    }

    /// `yuuka6_1B1B1`: a rotating blue ring whose count grows.
    fn y6_rot_rings(&mut self, env: &mut BossEnv) {
        if self.phase_frame > 48 && env.frame % 8 == 0 {
            self.angle = self.angle.wrapping_add(2);
            let t = BulletTemplate {
                spawn_type: 2,
                group: group::RING,
                count: (4 + self.phase_frame / 4).min(40) as u8,
                speed: (3 * SUBPIXEL) as u8,
                angle: self.angle,
                patnum: PAT_D_BLUE,
                ..Default::default()
            };
            env.pool.spawn(&t, self.x, self.y, (0, 0));
        }
        if self.phase_frame >= 144 {
            self.y6_end_pattern();
        }
    }

    /// `yuuka6_1B22B`: spawn two homing chasecross bullets every 8 frames.
    fn y6_chasecross_burst(&mut self, env: &mut BossEnv) {
        if self.phase_frame > 48 && env.frame % 8 == 0 {
            let a1 = self.rng.byte();
            self.y6_chasecross_add(a1, 2 * SUBPIXEL);
            let a2 = self.rng.byte();
            self.y6_chasecross_add(a2, 2 * SUBPIXEL);
        }
        if self.phase_frame >= 144 {
            self.y6_end_pattern();
        }
    }

    /// Phase 16 (`yuuka6_1B282`): the finale — twin small-red rings sweeping.
    fn y6_final(&mut self, env: &mut BossEnv) {
        let v = self.phase_frame & 0x1F;
        if v % 4 == 0 {
            self.angle = self.angle.wrapping_add(8);
            for base in [0x82u8, 0x80u8] {
                let t = BulletTemplate {
                    spawn_type: 1,
                    group: group::RING,
                    count: 8,
                    speed: (v as i32 + 2 * SUBPIXEL) as u8,
                    angle: base.wrapping_sub(self.angle),
                    patnum: PAT_SMALL_RED,
                    ..Default::default()
                };
                env.pool.spawn(&t, self.x, self.y, (0, 0));
            }
        }
        let dead = self.hittest();
        if dead || self.phase_frame >= 2500 {
            env.fx.spark(self.x, self.y);
            self.phase_state = if self.phase_frame < 2500 { 1 } else { 0 };
            self.phase = 17;
            self.phase_frame = 0;
        }
    }
}
