//! Stage 2 boss **Kurumi** — `@kurumi_update$qv` (th04_main.asm:19251) plus its
//! pattern subs (`kurumi_18A79`..`18F8B`) and the `kurumi_spawnrays_add` /
//! `kurumi_18A79` spawn-ray engine. HP 4800; thresholds 3300 → 2050 → 550 → 0.
//!
//! Phases: intro (invincible) → settle → barrage+spawn-rays → decelerating
//! aimed rings → stacked spawn-rays → final spiral → defeat. The signature is
//! the **spawn-ray**: a line grows out from Kurumi along a fixed direction and,
//! where its tip leaves the field, erupts into three accelerating aimed rings.
//!
//! The ray-burst rings use `BSM_SPEEDUP` and the phase-3 rings use
//! `BSM_DECELERATE_THEN_TURN` exactly (the elliptical sway amplitude is the only
//! eyeballed value).

use super::{Boss, BossEnv, Ray, PLAYFIELD_H, PLAYFIELD_W, SUBPIXEL};
use crate::bullet::{group, Bsm, BulletTemplate};
use crate::math::{cos8, sin8, vector2};

const PAT_BALL_BLUE: u8 = 2;

impl Boss {
    pub(super) fn kurumi_update(&mut self, env: &mut BossEnv) {
        match self.phase {
            0 => self.kurumi_phase0_intro(env),
            1 => self.kurumi_phase1_pause(env),
            2 => self.kurumi_phase2_barrage(env, false),
            3 => self.kurumi_phase3_decel(env),
            4 => self.kurumi_phase2_barrage(env, true),
            5 => self.kurumi_phase5_final(env),
            _ => self.defeat_frame += 1,
        }
    }

    /// Phase 0 (loc_19178): 320-frame invincible charge.
    fn kurumi_phase0_intro(&mut self, env: &mut BossEnv) {
        if self.phase_frame >= 288 && self.phase_frame % 8 == 0 {
            env.fx.gather(self.x, self.y);
        }
        self.phase_frame += 1;
        if self.phase_frame >= 320 {
            self.phase = 1;
            self.phase_frame = 0;
        }
        self.hittest_invincible();
    }

    /// Phase 1 (loc_1923B): 32-frame invincible pause → barrage.
    fn kurumi_phase1_pause(&mut self, env: &mut BossEnv) {
        self.hittest_invincible();
        if self.phase_frame >= 32 {
            self.phase_next(3300, env.pool);
            self.mode = 3;
            self.angle = 192;
        }
    }

    /// Elliptical sway (`kurumi_18B68`/`18BA7`); `cw` steps the angle forward.
    fn kurumi_move(&mut self, cw: bool) {
        let a = self.angle;
        self.x = 192 * SUBPIXEL + ((cos8(a) * (84 * SUBPIXEL)) >> 8);
        self.y = 80 * SUBPIXEL + ((sin8(a) * (16 * SUBPIXEL)) >> 8);
        self.angle = if cw { self.angle.wrapping_add(1) } else { self.angle.wrapping_sub(1) };
    }

    /// Launch a spawn-ray from `boss + (dx, -10px)` travelling along `angle`.
    fn kurumi_ray_add(&mut self, dx: i32, angle: u8) {
        if let Some(r) = self.rays.iter_mut().find(|r| r.flag == 0) {
            let (vx, vy) = vector2(angle, 16 * SUBPIXEL);
            let ox = self.x + dx;
            let oy = self.y - 10 * SUBPIXEL;
            *r = Ray { flag: 1, ox, oy, tx: ox, ty: oy, vx, vy };
        }
    }

    /// `kurumi_18A79`: advance every spawn-ray; when a tip leaves the field fire
    /// three accelerating aimed rings of `burst_count` there. Returns whether
    /// all rays are now free.
    fn kurumi_rays_step(&mut self, env: &mut BossEnv, burst_count: u8) -> bool {
        let (w, h) = (PLAYFIELD_W * SUBPIXEL, PLAYFIELD_H * SUBPIXEL);
        let mut bursts: Vec<(i32, i32)> = Vec::new();
        let mut free = 0;
        for r in self.rays.iter_mut() {
            match r.flag {
                0 => free += 1,
                1 => {
                    if r.tx > 0 && r.tx < w && r.ty > 0 && r.ty < h {
                        r.tx += r.vx;
                        r.ty += r.vy;
                    } else {
                        bursts.push((r.tx - r.vx, r.ty - r.vy));
                        r.flag = 2;
                    }
                }
                _ => {
                    if r.ox > 0 && r.ox < w && r.oy > 0 && r.oy < h {
                        r.ox += r.vx;
                        r.oy += r.vy;
                    } else {
                        r.flag = 0;
                    }
                }
            }
        }
        for (bx, by) in bursts {
            env.fx.circle_grow(bx, by);
            let mut speed = 2 * SUBPIXEL;
            for _ in 0..3 {
                let t = BulletTemplate {
                    group: group::RING_AIMED,
                    count: burst_count,
                    speed: speed as u8,
                    patnum: PAT_BALL_BLUE,
                    special: Bsm::Speedup,
                    speed_delta: 1,
                    ..Default::default()
                };
                env.pool.spawn(&t, bx, by, env.player);
                speed += 6;
            }
        }
        free == self.rays.len()
    }

    /// The expanding 22-ring blue burst fired when a barrage mode-0 window ends
    /// (loc_19279/loc_19387). `step` is the per-ring angle delta (−3 / +3).
    fn kurumi_ring_burst(&mut self, env: &mut BossEnv, step: i8) {
        let rings = (self.phase_state as i32).min(5).max(0);
        let mut angle = self.rng.byte();
        let mut speed = SUBPIXEL; // 1px
        for _ in 0..rings {
            let t = BulletTemplate {
                group: group::RING,
                count: 22,
                speed: speed as u8,
                angle,
                patnum: PAT_BALL_BLUE,
                ..Default::default()
            };
            env.pool.spawn(&t, self.x, self.y, (0, 0));
            speed += 8;
            angle = angle.wrapping_add(step as u8);
        }
    }

    /// A spawn-ray attack window (loc_18BE6 etc.): telegraph, launch rays at the
    /// given (frame, dx, angle) schedule, then run the ray engine; resets `mode`
    /// to 0 once every ray is spent. `next_end` is the phase HP threshold.
    fn kurumi_ray_pattern(
        &mut self,
        env: &mut BossEnv,
        launches: &[(i32, i32, u8)],
        burst_count: u8,
    ) {
        if self.phase_frame == 48 {
            env.fx.circle_shrink(self.x - 12 * SUBPIXEL, self.y - 10 * SUBPIXEL);
            env.fx.circle_shrink(self.x + 12 * SUBPIXEL, self.y - 10 * SUBPIXEL);
        }
        for &(frame, dx, angle) in launches {
            if self.phase_frame == frame {
                let jitter = self.rng.and(15) as u8;
                self.kurumi_ray_add(dx, angle.wrapping_add(if dx < 0 { jitter.wrapping_neg() } else { jitter }));
            }
        }
        if self.phase_frame > 64 && self.kurumi_rays_step(env, burst_count) {
            self.phase_frame = 0;
            self.mode = 0;
        }
    }

    /// Phases 2 & 4 (loc_19264 / loc_19372): circular movement, then a random
    /// pattern — either the 22-ring burst (mode 0) or a spawn-ray attack. The
    /// `late` flag selects phase-4 parameters (angle step +3, fewer ray bursts).
    fn kurumi_phase2_barrage(&mut self, env: &mut BossEnv, late: bool) {
        let (next_end, burst_step) = if late { (0, 3i8) } else { (2050, -3i8) };
        match self.mode {
            0 => {
                self.kurumi_move(true);
                if self.phase_frame >= 96 {
                    self.phase_frame = 0;
                    self.phase_state += 1;
                    if self.phase_state <= 10 {
                        self.kurumi_ring_burst(env, burst_step);
                    }
                    let pool_modes = if late { 3 } else { 4 };
                    self.mode = (self.rng.modulo(pool_modes) as i16) + 1;
                }
            }
            // Spawn-ray attacks. Phase 2: left / right / dual (burst 16/16/8).
            // Phase 4: stacked left / right / dual (burst 12/12/6).
            1 if !late => self.kurumi_ray_pattern(env, &[(64, -12 * SUBPIXEL, 0x18)], 16),
            2 if !late => self.kurumi_ray_pattern(env, &[(64, 12 * SUBPIXEL, 0x68)], 16),
            _ if !late => self.kurumi_ray_pattern(
                env,
                &[(64, 12 * SUBPIXEL, 0x68), (64, -12 * SUBPIXEL, 0x18)],
                8,
            ),
            1 => self.kurumi_ray_pattern(
                env,
                &[(64, -12 * SUBPIXEL, 0x18), (80, -12 * SUBPIXEL, 0x10), (96, -12 * SUBPIXEL, 0x08)],
                12,
            ),
            2 => self.kurumi_ray_pattern(
                env,
                &[(64, 12 * SUBPIXEL, 0x68), (80, 12 * SUBPIXEL, 0x70), (96, 12 * SUBPIXEL, 0x78)],
                12,
            ),
            _ => self.kurumi_ray_pattern(
                env,
                &[
                    (64, -12 * SUBPIXEL, 0x18),
                    (64, 12 * SUBPIXEL, 0x68),
                    (80, -12 * SUBPIXEL, 0x10),
                    (80, 12 * SUBPIXEL, 0x70),
                    (96, -12 * SUBPIXEL, 0x08),
                    (96, 12 * SUBPIXEL, 0x78),
                ],
                6,
            ),
        }
        if self.hittest() {
            self.phase_next(next_end, env.pool);
            if !late {
                self.mode = 0;
            }
        }
    }

    /// Phase 3 (loc_19328): counter-rotate while spamming aimed rings that
    /// decelerate to 0 then turn ±0x40 (`BSM_DECELERATE_THEN_TURN`). Ends on the
    /// HP threshold or a 2000-frame timeout.
    fn kurumi_phase3_decel(&mut self, env: &mut BossEnv) {
        match self.mode {
            0 => {
                self.kurumi_move(false);
                if self.phase_frame >= 128 {
                    self.phase_frame = 0;
                    self.mode = 1;
                }
            }
            _ => {
                self.kurumi_move(false);
                if env.frame % 57 == 0 {
                    for (speed, turn) in [(3 * SUBPIXEL, 0x40u8), (2 * SUBPIXEL, 0xC0u8)] {
                        let t = BulletTemplate {
                            group: group::RING_AIMED,
                            count: 16,
                            speed: speed as u8,
                            patnum: PAT_BALL_BLUE,
                            special: Bsm::DecelThenTurn,
                            turn_arg: turn,
                            turns_max: 1,
                            ..Default::default()
                        };
                        env.pool.spawn(&t, self.x, self.y, env.player);
                    }
                }
            }
        }
        if self.phase_frame > 2000 || self.hittest() {
            self.phase_next(550, env.pool);
        }
    }

    /// Phase 5 (loc_19431): final spiral — return to centre and fire a twin
    /// rotating stream from ±12px plus a periodic aimed 5-spread. Ends on HP=0 or
    /// a 700-frame timeout (`kurumi_1905A`, approximated).
    fn kurumi_phase5_final(&mut self, env: &mut BossEnv) {
        // Drift back toward the home position.
        self.vx = if self.x < 191 * SUBPIXEL { 24 } else if self.x > 193 * SUBPIXEL { -24 } else { 0 };
        self.vy = if self.y < 79 * SUBPIXEL { 12 } else if self.y > 81 * SUBPIXEL { -12 } else { 0 };
        self.move_step();

        if self.phase_frame == 16 {
            self.sb[0] = -0x20 - self.rng.and(15) as i32; // right stream angle
            self.sb[1] = -0x60 + self.rng.and(15) as i32; // left stream angle
        }
        if self.phase_frame % 8 == 0 {
            for (i, (off, track)) in [(12, 0usize), (-12, 1usize)].iter().enumerate() {
                let _ = i;
                let mut speed = SUBPIXEL;
                for _ in 0..3 {
                    let t = BulletTemplate {
                        group: group::SINGLE,
                        count: 1,
                        speed: speed as u8,
                        angle: self.sb[*track] as u8,
                        origin_x: (off * SUBPIXEL) as i16,
                        origin_y: (-10 * SUBPIXEL) as i16,
                        patnum: PAT_BALL_BLUE,
                        ..Default::default()
                    };
                    env.pool.spawn(&t, self.x, self.y, (0, 0));
                    speed += 8;
                }
            }
            self.sb[0] += 0x10;
            self.sb[1] -= 0x10;
        }
        if self.phase_frame % 48 == 0 {
            let t = BulletTemplate {
                group: group::SPREAD_AIMED,
                count: 5,
                delta: 9,
                speed: (2 * SUBPIXEL) as u8,
                ..Default::default()
            };
            env.pool.spawn(&t, self.x, self.y, env.player);
        }

        if self.phase_frame > 700 || self.hittest() {
            self.phase_state = if self.phase_frame > 700 { 0 } else { 1 };
            env.fx.spark(self.x, self.y);
            self.phase = 6;
            self.begin_defeat(env.pool);
        }
    }
}
