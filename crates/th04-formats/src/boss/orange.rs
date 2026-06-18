//! Stage 1 boss **Orange** — `@orange_update$qv` (th04_main.asm:20254) plus its
//! pattern subs `orange_195E4/19686/19720/197BB/19814/19878/1998B`. Six phases:
//! charge → triple-ring → random barrage (4 sub-patterns) → wandering spray →
//! rotating spiral → defeat. HP 3050, thresholds 1950 → 450 → 0.

use super::{Boss, BossEnv, SUBPIXEL};
use crate::bullet::{group, BulletTemplate};
use crate::math::{iatan2, sin8};

/// `PAT_BULLET16_N_OUTLINED_BALL_WHITE` — the white 16×16 ball; the index is
/// mapped at render time, here it just tags the larger bullets.
const PAT_BALL_WHITE: u8 = 1;

impl Boss {
    pub(super) fn orange_update(&mut self, env: &mut BossEnv) {
        match self.phase {
            0 => self.orange_phase0_intro(env),
            1 => self.orange_phase1_rings(env),
            2 => self.orange_phase2_barrage(env),
            3 => self.orange_phase3_wander(env),
            4 => self.orange_phase4_final(env),
            _ => self.defeat_frame_tick(),
        }
    }

    fn defeat_frame_tick(&mut self) {
        self.defeat_frame += 1;
    }

    /// Phase 0 (loc_19AC8): 352-frame charge. Vulnerable, fires nothing.
    fn orange_phase0_intro(&mut self, env: &mut BossEnv) {
        self.phase_frame += 1;
        if self.phase_frame >= 320 && self.phase_frame % 8 == 0 {
            env.fx.gather(self.x, self.y); // charge telegraph
        }
        if self.phase_frame >= 352 {
            self.phase = 1;
            self.phase_frame = 0;
            self.pattern_num_prev = -1;
            self.patterns_done = 0;
        }
        self.hittest_damage();
    }

    /// Phase 1 (loc_19B88): wait 32 frames, then 3 concentric 16-rings at speeds
    /// 4/3/2 px, and enter the barrage phase.
    fn orange_phase1_rings(&mut self, env: &mut BossEnv) {
        self.phase_frame += 1;
        if self.phase_frame >= 32 {
            self.phase = 2;
            self.phase_frame = 0;
            self.mode = 0;
            let mut speed: i32 = 4 * SUBPIXEL;
            for _ in 0..3 {
                let t = BulletTemplate {
                    group: group::RING,
                    count: 16,
                    speed: speed as u8,
                    ..Default::default()
                };
                env.pool.spawn(&t, self.x, self.y, (0, 0));
                speed -= SUBPIXEL;
            }
        }
        self.hittest_damage();
    }

    /// Phase 2 (loc_19C10): pick a random sub-pattern (modes 1-4), run it until
    /// it resets `mode` to 0; after 16 patterns or HP threshold → phase 3.
    fn orange_phase2_barrage(&mut self, env: &mut BossEnv) {
        match self.mode {
            0 => {
                self.phase_frame = 0;
                loop {
                    self.mode = (self.rng.and(3) as i16) + 1;
                    if self.mode != self.pattern_num_prev {
                        break;
                    }
                }
                self.pattern_num_prev = self.mode;
                self.patterns_done += 1;
                if self.patterns_done >= 16 {
                    self.phase_next(450, env.pool);
                    return;
                }
            }
            1 => self.orange_p_twinsweep(env),
            2 => self.orange_p_aimed_spread(env),
            3 => self.orange_p_aimed_ring(env),
            _ => self.orange_p_twin_rings(env),
        }
        if self.hittest() {
            self.phase_next(450, env.pool);
        }
    }

    /// Phase 3 (loc_19C8A): 128-frame pause then the wandering random spray;
    /// ends on HP threshold (450 → 0) or a 1500-frame timeout.
    fn orange_phase3_wander(&mut self, env: &mut BossEnv) {
        match self.mode {
            0 => {
                if self.phase_frame > 128 {
                    self.phase_frame = 0;
                    self.mode = 1;
                }
            }
            _ => self.orange_p_wander(env),
        }
        if self.phase_frame > 1500 || self.hittest() {
            self.phase_next(0, env.pool);
        }
    }

    /// Phase 4 (loc_19CF0): return to centre (mode 0, 128 frames) then the final
    /// rotating spiral (mode 1). Ends on HP=0 (bonus) or a 600-frame timeout.
    fn orange_phase4_final(&mut self, env: &mut BossEnv) {
        match self.mode {
            0 => {
                if self.phase_frame > 128 {
                    self.phase_frame = 0;
                    self.mode = 1;
                }
                self.vx = if self.x < 191 * SUBPIXEL {
                    24
                } else if self.x > 193 * SUBPIXEL {
                    -24
                } else {
                    self.vx
                };
                self.vy = if self.y < 79 * SUBPIXEL {
                    12
                } else if self.y > 81 * SUBPIXEL {
                    -12
                } else {
                    self.vy
                };
                if self.phase_frame == 96 {
                    env.fx.gather(self.x, self.y);
                }
                if self.phase_frame == 112 {
                    env.fx.circle_shrink(self.x, self.y);
                }
                self.move_step();
            }
            _ => self.orange_p_spiral(env),
        }
        let timed_out = self.phase_frame > 600;
        let depleted = !timed_out && self.hittest();
        if timed_out || depleted {
            self.phase_state = if timed_out { 0 } else { 1 };
            self.phase = 5;
            self.phase_frame = 0;
            self.mode = 0;
            env.fx.spark(self.x, self.y);
            self.begin_defeat(env.pool);
        }
    }

    // --- Phase-2 sub-patterns (each gated by `orange_gate`) ---------------

    /// `orange_195E4`: shared move-to-random-spot + telegraph gate. Returns
    /// 0 (moving), 1 (idle), or 2 (fire now).
    fn orange_gate(&mut self, env: &mut BossEnv) -> i32 {
        let f = self.phase_frame;
        if f < 16 {
            return 1;
        }
        if f == 16 {
            let tx = self.rng.modulo(320 * SUBPIXEL) + 32 * SUBPIXEL;
            let ty = self.rng.modulo(96 * SUBPIXEL) + 64 * SUBPIXEL;
            self.vx = (tx - self.x) / (4 * SUBPIXEL);
            self.vy = (ty - self.y) / (4 * SUBPIXEL);
            env.fx.gather(self.x, self.y);
        }
        if f < 70 {
            self.move_step();
            return 0;
        }
        if f == 70 {
            env.fx.circle_shrink(self.x, self.y);
            return 0;
        }
        if f >= 86 {
            return 2;
        }
        1
    }

    /// Mode 1 (`orange_19686`): twin sweeping pellet pairs.
    fn orange_p_twinsweep(&mut self, env: &mut BossEnv) {
        if self.orange_gate(env) != 2 {
            return;
        }
        if self.phase_frame == 86 {
            let right = self.rng.and(1) != 0;
            self.angle = if right { 0x80 } else { 0x00 };
            self.phase_state = if right { -0x0B } else { 0x0B };
        }
        if env.frame % 2 == 0 {
            let t = BulletTemplate {
                group: group::RING,
                count: 2,
                speed: (SUBPIXEL + 14) as u8,
                angle: self.angle,
                ..Default::default()
            };
            env.pool.spawn(&t, self.x, self.y, (0, 0));
            let t2 = BulletTemplate {
                angle: self.angle.wrapping_add(5),
                speed: (SUBPIXEL + 4) as u8,
                ..t
            };
            env.pool.spawn(&t2, self.x, self.y, (0, 0));
            self.angle = self.angle.wrapping_add(self.phase_state as u8);
        }
        if self.phase_frame >= 118 {
            self.mode = 0;
        }
    }

    /// Mode 2 (`orange_19720`): twin aimed 3-way spreads, speed ramping.
    fn orange_p_aimed_spread(&mut self, env: &mut BossEnv) {
        if self.orange_gate(env) != 2 {
            return;
        }
        if self.phase_frame == 86 {
            self.angle = iatan2(env.player.1 - self.y, env.player.0 - self.x);
            self.phase_state = SUBPIXEL as i16; // reuse phase_state as the ramping speed
        }
        if env.frame % 4 == 0 {
            let speed = self.phase_state as u8;
            let t = BulletTemplate {
                group: group::SPREAD,
                count: 3,
                delta: 0x0C,
                speed,
                angle: self.angle.wrapping_sub(0x20),
                patnum: PAT_BALL_WHITE,
                ..Default::default()
            };
            env.pool.spawn(&t, self.x, self.y, (0, 0));
            let t2 = BulletTemplate { angle: self.angle.wrapping_add(0x20), ..t };
            env.pool.spawn(&t2, self.x, self.y, (0, 0));
            self.phase_state = self.phase_state.wrapping_add(6);
        }
        if self.phase_frame >= 118 {
            self.mode = 0;
        }
    }

    /// Mode 3 (`orange_197BB`): 16-way aimed ring every 8 frames.
    fn orange_p_aimed_ring(&mut self, env: &mut BossEnv) {
        if self.orange_gate(env) != 2 {
            return;
        }
        if env.frame % 8 == 0 {
            let t = BulletTemplate {
                group: group::RING_AIMED,
                count: 16,
                speed: (2 * SUBPIXEL) as u8,
                patnum: PAT_BALL_WHITE,
                ..Default::default()
            };
            env.pool.spawn(&t, self.x, self.y, env.player);
        }
        if self.phase_frame >= 118 {
            self.mode = 0;
        }
    }

    /// Mode 4 (`orange_19814`): two 8-rings offset ±32px, angle creeping.
    fn orange_p_twin_rings(&mut self, env: &mut BossEnv) {
        if self.orange_gate(env) != 2 {
            return;
        }
        if env.frame % 8 == 0 {
            self.angle = self.angle.wrapping_add(8);
            let t = BulletTemplate {
                group: group::RING,
                count: 8,
                speed: (SUBPIXEL + 14) as u8,
                angle: self.angle,
                origin_x: (-32 * SUBPIXEL) as i16,
                ..Default::default()
            };
            env.pool.spawn(&t, self.x, self.y, (0, 0));
            let t2 = BulletTemplate { origin_x: (32 * SUBPIXEL) as i16, ..t };
            env.pool.spawn(&t2, self.x, self.y, (0, 0));
        }
        if self.phase_frame >= 118 {
            self.mode = 0;
        }
    }

    /// Phase-3 pattern (`orange_19878`): wander left/right + vertical bob,
    /// bouncing off the walls, spraying random-angle bullets from two origins.
    fn orange_p_wander(&mut self, env: &mut BossEnv) {
        if self.phase_frame == 1 {
            self.vx = if self.x < 192 * SUBPIXEL { SUBPIXEL } else { -SUBPIXEL };
            self.angle = 0;
        }
        self.vy = (sin8(self.angle) * SUBPIXEL) >> 8;
        if self.y >= 96 * SUBPIXEL {
            self.vy = -SUBPIXEL;
        }
        if self.y <= 48 * SUBPIXEL {
            self.vy = SUBPIXEL;
        }
        self.angle = self.angle.wrapping_add(2);
        let x = self.move_step();
        if x <= 32 * SUBPIXEL || x >= 352 * SUBPIXEL {
            self.vx = -self.vx;
        }
        if env.frame % 4 != 0 {
            return;
        }
        let count = if self.hp <= 700 { 2 } else { 1 };
        for side in [-32, 32] {
            let t = BulletTemplate {
                group: group::RANDOM_ANGLE,
                count,
                speed: (2 * SUBPIXEL) as u8,
                origin_x: (side * SUBPIXEL) as i16,
                patnum: PAT_BALL_WHITE,
                ..Default::default()
            };
            env.pool.spawn(&t, self.x, self.y, (0, 0));
        }
    }

    /// Phase-4 final spell (`orange_1998B`): a rotating arm that gains more arms
    /// as the spell progresses, plus an outward burst late on.
    fn orange_p_spiral(&mut self, env: &mut BossEnv) {
        if env.frame % 4 != 0 {
            return;
        }
        let (ox, oy) = (8 * SUBPIXEL, -16 * SUBPIXEL);
        self.angle = self.angle.wrapping_sub(7);
        let speed = (2 * SUBPIXEL) as u8;
        let mut a = self.angle;
        self.orange_fire_single(env, a, speed, 0, ox, oy); // arm A
        let f = self.phase_frame;
        if f < 128 {
            return;
        }
        if f < 192 {
            a = a.wrapping_add(0x40);
            self.orange_fire_single(env, a, speed, 0, ox, oy);
        } else if f < 256 {
            for _ in 0..2 {
                a = a.wrapping_add(0x40);
                self.orange_fire_single(env, a, speed, 0, ox, oy);
            }
        } else if f < 320 {
            for _ in 0..3 {
                a = a.wrapping_add(0x40);
                self.orange_fire_single(env, a, speed, 0, ox, oy);
            }
        } else {
            for _ in 0..3 {
                a = a.wrapping_add(0x40);
                self.orange_fire_single(env, a, speed, 0, ox, oy);
            }
            let fast = speed.wrapping_add(SUBPIXEL as u8);
            a = (a as i8).wrapping_neg() as u8;
            a = a.wrapping_sub(0x20);
            self.orange_fire_single(env, a, fast, PAT_BALL_WHITE, ox, oy);
            a = a.wrapping_add(0x80);
            self.orange_fire_single(env, a, fast, PAT_BALL_WHITE, ox, oy);
        }
    }

    fn orange_fire_single(&self, env: &mut BossEnv, angle: u8, speed: u8, patnum: u8, ox: i32, oy: i32) {
        let t = BulletTemplate {
            group: group::SINGLE,
            count: 1,
            speed,
            angle,
            patnum,
            origin_x: ox as i16,
            origin_y: oy as i16,
            ..Default::default()
        };
        env.pool.spawn(&t, self.x, self.y, (0, 0));
    }
}
