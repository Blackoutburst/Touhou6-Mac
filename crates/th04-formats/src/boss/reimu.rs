//! Stage 4 rival **Reimu** — `@reimu_update$qv` (th04_main.asm:27309) + its orb
//! engine (`reimu_1EBF3`/`orbs_add_spinning`/`orbs_add_moving`) and mode subs
//! (`reimu_1ED15`..`1F378`). The fight when playing as Marisa. HP 9100.
//!
//! Ported exactly: the orb physics (spin out to a 64px radius while rotating for
//! `spin_time`, then convert to a free-flying bullet that bounces off the side
//! walls under gravity), the 13-phase flow (attack phases 2/4/6/8/9 at HP gates
//! 7900/6300/4500/2700/900/0, interleaved with move-transitions 3/5/7/10 and the
//! final spell 11), the jink movement, and every mode's danmaku.
//!
//! Approximations: a few `boss_statebyte` setup values (orb count, spread/stack
//! counts, turn limits) aren't in the update and are set here to their Normal
//! values; the exact `randring2` sequence is the documented engine-wide
//! deviation. Bullet special motions are now exact (see `bullet.rs`).

use super::{Boss, BossEnv, Orbit, PLAYFIELD_H, PLAYFIELD_W, SUBPIXEL};
use crate::bullet::{group, Bsm, BulletTemplate};
use crate::math::{cos8, iatan2, sin8, vector2};

const PAT_BALL_BLUE: u8 = 2;
const PAT_BALL_RED: u8 = 6;
const PAT_ORB_YELLOW: u8 = 5;

// boss_statebyte setup values (Normal rank).
const ORB_COUNT: i32 = 8;
const ORB_INTERVAL: i32 = 16; // statebyte[1]
const SPREAD_TURNS_MAX: u8 = 1; // statebyte[2]
const SPREAD3_COUNT: u8 = 3; // statebyte[3]
const SPREAD3_DELTA: u8 = 8; // statebyte[4]
const SPREAD6_DELTA: u8 = 6; // statebyte[5]
const STACK_COUNT: u8 = 3; // statebyte[6]
const ORB_MOVE_SPEED: i32 = 0x38;

impl Boss {
    pub(super) fn reimu_update(&mut self, env: &mut BossEnv) {
        match self.phase {
            0 => {
                self.phase_frame += 1;
                self.hittest_invincible();
                if self.phase_frame > 96 {
                    self.phase = 1;
                    self.phase_frame = 0;
                }
            }
            1 => {
                self.phase_frame += 1;
                self.hittest_invincible();
                if self.phase_frame >= 128 {
                    self.phase_end_hp = 9100;
                    self.phase_next(7900, env.pool); // → phase 2
                }
            }
            2 => self.reimu_phase2(env),
            3 => {
                if self.reimu_move(true, 96 * SUBPIXEL) {
                    self.phase = 4;
                    self.reimu_open_orbs(4, PAT_BALL_BLUE);
                    self.sb[2] = 0; // subpattern_id
                }
            }
            4 => self.reimu_phase4(env),
            5 => {
                if self.reimu_move(false, 128 * SUBPIXEL) {
                    self.phase = 6;
                    self.reimu_open_orbs(4, PAT_BALL_BLUE);
                }
            }
            6 => self.reimu_phase6(env),
            7 => {
                if self.reimu_move(true, 96 * SUBPIXEL) {
                    self.phase = 8;
                    self.reimu_open_orbs(0x12, PAT_ORB_YELLOW);
                }
            }
            8 => self.reimu_phase8(env),
            9 => self.reimu_phase9(env),
            10 => {
                if self.reimu_move(true, 96 * SUBPIXEL) {
                    self.phase = 11;
                    self.reimu_open_orbs(4, PAT_BALL_BLUE);
                }
            }
            11 => self.reimu_phase11(env),
            _ => {
                // Phase 12: defeat sequence.
                self.phase_frame += 1;
                if self.phase_frame >= 32 {
                    env.fx.spark(self.x, self.y);
                    self.begin_defeat(env.pool);
                }
            }
        }
        self.reimu_orbs_step();
    }

    fn reimu_open_orbs(&mut self, angle_speed: i32, patnum: u8) {
        self.phase_frame = 0;
        self.phase_state = 0;
        self.mode = 0;
        self.sb[0] = angle_speed;
        self.sb[1] = patnum as i32;
    }

    /// Move-transition phase: servo to (192, `ty`) over 64 vulnerable frames.
    fn reimu_move(&mut self, servo_x: bool, ty: i32) -> bool {
        if servo_x {
            self.x += if self.x < 192 * SUBPIXEL {
                2 * SUBPIXEL
            } else if self.x > 192 * SUBPIXEL {
                -2 * SUBPIXEL
            } else {
                0
            };
        }
        self.y += if self.y < ty {
            SUBPIXEL
        } else if self.y > ty {
            -SUBPIXEL
        } else {
            0
        };
        self.hittest();
        self.phase_frame >= 64
    }

    // === Orb engine =======================================================

    /// `orbs_add_spinning`: fill up to `count` free orbs with spin-out orbs.
    fn reimu_orbs_add_spinning(&mut self, count: i32, angle_offset: u8, spin_time: i32) {
        let (ox, oy) = (self.x, self.y);
        let aspeed = self.sb[0] as i8;
        let patnum = self.sb[1] as u8;
        let mut i = 0i32;
        for o in self.orbits.iter_mut() {
            if i >= count {
                break;
            }
            if o.flag == 0 {
                *o = Orbit {
                    flag: 1,
                    angle: (((i * 0x100) / count) as u8).wrapping_add(angle_offset),
                    aspeed,
                    cx: ox,
                    cy: oy,
                    ox,
                    oy,
                    dist: 0,
                    spin_time,
                    mspeed: ORB_MOVE_SPEED,
                    patnum,
                    vx: 0,
                    vy: 0,
                    hp: 0,
                };
                i += 1;
            }
        }
    }

    /// `orbs_add_moving`: launch one free orb straight at `angle`.
    fn reimu_orb_add_moving(&mut self, angle: u8) {
        let (cx, cy) = (self.x, self.y);
        let patnum = self.sb[1] as u8;
        if let Some(o) = self.orbits.iter_mut().find(|o| o.flag == 0) {
            let (vx, vy) = vector2(angle, ORB_MOVE_SPEED);
            *o = Orbit {
                flag: 2,
                angle,
                aspeed: self.sb[0] as i8,
                cx,
                cy,
                ox: cx,
                oy: cy,
                dist: 0,
                spin_time: 0,
                mspeed: ORB_MOVE_SPEED,
                patnum,
                vx,
                vy,
                hp: 0,
            };
        }
    }

    /// `reimu_1EBF3`: advance every orb.
    fn reimu_orbs_step(&mut self) {
        for o in self.orbits.iter_mut() {
            match o.flag {
                1 => {
                    o.cx = o.ox + ((cos8(o.angle) * o.dist) >> 8);
                    o.cy = o.oy + ((sin8(o.angle) * o.dist) >> 8);
                    if o.dist < 64 * SUBPIXEL {
                        o.dist += 4 * SUBPIXEL;
                    }
                    o.spin_time -= 1;
                    o.angle = o.angle.wrapping_add(o.aspeed as u8);
                    if o.spin_time <= 0 {
                        let turn: u8 = if o.aspeed >= 0 { 0x40 } else { 0xC0 };
                        o.angle = o.angle.wrapping_add(turn);
                        let (vx, vy) = vector2(o.angle, o.mspeed);
                        o.vx = vx;
                        o.vy = vy;
                        o.flag = 2;
                    }
                }
                2 => {
                    o.cx += o.vx;
                    if o.cx < 0 || o.cx > PLAYFIELD_W * SUBPIXEL {
                        o.vx = -o.vx;
                    }
                    o.cy += o.vy;
                    if o.cy >= PLAYFIELD_H * SUBPIXEL {
                        o.flag = 0;
                    }
                    o.vy += 1; // gravity
                }
                _ => {}
            }
        }
    }

    // === Shared helpers ===================================================

    /// `reimu_1EA4B`: telegraph gate — 0 before frame 46, 2 at 46, 1 after.
    fn reimu_gate(&mut self, env: &mut BossEnv) -> i32 {
        let pf = self.phase_frame;
        if pf == 14 {
            env.fx.gather(self.x + 4 * SUBPIXEL, self.y - 28 * SUBPIXEL);
        }
        if pf == 0x16 {
            env.fx.circle_shrink(self.x + 4 * SUBPIXEL, self.y - 28 * SUBPIXEL);
        }
        match pf.cmp(&46) {
            std::cmp::Ordering::Less => 0,
            std::cmp::Ordering::Equal => 2,
            std::cmp::Ordering::Greater => 1,
        }
    }

    /// Bullet origin used by most modes (`boss + (4, -28)`).
    fn reimu_bo(&self) -> (i32, i32) {
        (self.x + 4 * SUBPIXEL, self.y - 28 * SUBPIXEL)
    }

    /// A random origin within ±32px of the boss.
    fn reimu_random_origin(&mut self) -> (i32, i32) {
        let ox = self.x - 32 * SUBPIXEL + self.rng.modulo(64 * SUBPIXEL);
        let oy = self.y - 32 * SUBPIXEL + self.rng.modulo(64 * SUBPIXEL);
        (ox, oy)
    }

    /// jink movement (`reimu_1E917`/`1E9B1`); returns once the leg completes.
    /// `flip` selects the `1E9B1` variant (phase 6).
    fn reimu_jink(&mut self, flip: bool) {
        if self.phase_frame == 1 {
            let (vx, vy) = match (self.phase_state % 3, flip) {
                (0, false) => (-4 * SUBPIXEL, SUBPIXEL),
                (1, false) => (4 * SUBPIXEL, 0),
                (_, false) => (-4 * SUBPIXEL, -SUBPIXEL),
                (0, true) => (4 * SUBPIXEL, -SUBPIXEL),
                (1, true) => (-4 * SUBPIXEL, 0),
                (_, true) => (4 * SUBPIXEL, SUBPIXEL),
            };
            self.vx = vx;
            self.vy = vy;
        }
        self.x += self.vx;
        self.y += self.vy;
        let dur = if self.phase_state % 3 == 1 { 0x40 } else { 0x20 };
        if self.phase_frame == dur {
            self.phase_state += 1;
            self.mode = self.phase_state % 2;
            self.phase_frame = 0;
        }
    }

    /// The end-of-frame advance check shared by attack phases.
    fn reimu_attack_tail(&mut self, gate: i16, next_end: i32, env: &mut BossEnv) -> bool {
        if self.phase_state >= gate {
            self.phase_next(next_end, env.pool);
            true
        } else if self.hittest() {
            self.phase_next(next_end, env.pool);
            true
        } else {
            false
        }
    }

    // === Attack phases ====================================================

    fn reimu_phase2(&mut self, env: &mut BossEnv) {
        match self.mode {
            0 => self.reimu_m_aimed_spread6(env),
            1 => self.reimu_m_decel_cloud9(env),
            _ => self.reimu_jink(false),
        }
        self.reimu_attack_tail(9, 6300, env);
    }

    fn reimu_phase4(&mut self, env: &mut BossEnv) {
        match self.mode {
            0 | 1 | 2 => self.reimu_m_spinning_orbs(env),
            3 => self.reimu_m_chaos_rings(env),
            _ => {
                // loc_1F547: subpattern cycle.
                self.phase_state += 1;
                if self.sb[2] <= 2 {
                    if self.rng.and(1) != 0 {
                        self.sb[2] += 1;
                    } else {
                        self.sb[2] = 3;
                    }
                } else {
                    self.sb[2] = 0;
                }
                self.mode = self.sb[2] as i16;
                self.phase_frame = 0;
            }
        }
        self.reimu_attack_tail(18, 4500, env);
    }

    fn reimu_phase6(&mut self, env: &mut BossEnv) {
        match self.mode {
            0 => self.reimu_m_random_spray(env),
            1 => self.reimu_m_aimed_speedup(env),
            _ => self.reimu_jink(true),
        }
        self.reimu_attack_tail(11, 2700, env);
    }

    fn reimu_phase8(&mut self, env: &mut BossEnv) {
        match self.mode {
            0 => self.reimu_m_moving_orbs(env),
            1 => self.reimu_m_alt_spread(env),
            _ => {
                self.phase_state += 1;
                self.mode = self.phase_state & 1;
                self.phase_frame = 0;
            }
        }
        if self.reimu_attack_tail(10, 900, env) {
            self.sb[0] = 3; // orb angle_speed for the next phase
        }
    }

    fn reimu_phase9(&mut self, env: &mut BossEnv) {
        match self.mode {
            0 => self.reimu_m_random_spray(env),
            1 => self.reimu_m_stack_fan(env),
            _ => self.reimu_jink(false),
        }
        if self.reimu_attack_tail(12, 0, env) {
            self.sb[0] = 3;
        }
    }

    fn reimu_phase11(&mut self, env: &mut BossEnv) {
        self.reimu_m_final(env);
        let dead = self.hittest();
        if dead || self.phase_frame >= 1000 {
            env.fx.spark(self.x, self.y);
            self.phase_state = if self.phase_frame < 1000 { 1 } else { 0 };
            self.phase = 12;
            self.phase_frame = 0;
        }
    }

    // === Modes ============================================================

    /// `reimu_1ED15`: aimed 6-way yellow spread that drifts.
    fn reimu_m_aimed_spread6(&mut self, env: &mut BossEnv) {
        let (bx, by) = self.reimu_bo();
        match self.reimu_gate(env) {
            2 => {
                self.angle = iatan2(env.player.1 - self.y, env.player.0 - self.x);
                self.sb[4] = if env.player.0 < 192 * SUBPIXEL { -2 } else { 2 };
            }
            1 => {
                if self.phase_frame % 4 == 0 {
                    let t = BulletTemplate {
                        spawn_type: 2,
                        group: group::SPREAD,
                        count: 6,
                        delta: SPREAD6_DELTA,
                        speed: (6 * SUBPIXEL) as u8,
                        angle: self.angle,
                        patnum: PAT_BALL_BLUE,
                        ..Default::default()
                    };
                    env.pool.spawn(&t, bx, by, (0, 0));
                    if self.phase_frame >= 64 {
                        self.angle = self.angle.wrapping_add(self.sb[4] as u8);
                    }
                }
                if self.phase_frame >= 112 {
                    self.mode = -1;
                    self.phase_frame = 0;
                }
            }
            _ => {}
        }
    }

    /// `reimu_1EDBC`: aimed 9-way cloud spread that decelerates then turns.
    fn reimu_m_decel_cloud9(&mut self, env: &mut BossEnv) {
        let (bx, by) = self.reimu_bo();
        match self.reimu_gate(env) {
            2 => {
                let t = BulletTemplate {
                    spawn_type: 4,
                    group: group::SPREAD_AIMED,
                    count: 9,
                    delta: 6,
                    speed: (5 * SUBPIXEL + 5) as u8,
                    patnum: PAT_BALL_BLUE,
                    special: Bsm::DecelThenTurnAimed,
                    turns_max: SPREAD_TURNS_MAX,
                    ..Default::default()
                };
                env.pool.spawn(&t, bx, by, env.player);
            }
            1 => {
                if self.phase_frame >= 128 {
                    self.mode = -1;
                    self.phase_frame = 0;
                }
            }
            _ => {}
        }
    }

    /// `reimu_1EE21`: spawn a ring of spinning orbs.
    fn reimu_m_spinning_orbs(&mut self, _env: &mut BossEnv) {
        if self.phase_frame == 32 {
            let off = self.rng.byte();
            self.reimu_orbs_add_spinning(ORB_COUNT, off, 64);
        }
        if self.phase_frame >= 96 {
            self.phase_frame = 0;
            self.mode = -1;
            self.sb[0] = -self.sb[0];
        }
    }

    /// `reimu_1EE73`: random-origin speeding rings + 12-way pellet-stack fans.
    fn reimu_m_chaos_rings(&mut self, env: &mut BossEnv) {
        if self.reimu_gate(env) == 1 {
            if self.phase_frame % 32 == 0 {
                let (ox, oy) = self.reimu_random_origin();
                let t = BulletTemplate {
                    spawn_type: 5,
                    group: group::RING,
                    count: 16,
                    speed: SUBPIXEL as u8,
                    patnum: PAT_BALL_BLUE,
                    special: Bsm::Speedup,
                    speed_delta: 1,
                    ..Default::default()
                };
                env.pool.spawn(&t, ox, oy, (0, 0));
            }
            if self.phase_frame % 32 == 16 {
                let (ox, oy) = self.reimu_random_origin();
                let mut angle = self.rng.byte();
                for _ in 0..12 {
                    let t = BulletTemplate {
                        spawn_type: 1,
                        group: group::STACK,
                        count: 4,
                        delta: 8,
                        speed: (SUBPIXEL + 8) as u8,
                        angle,
                        ..Default::default()
                    };
                    env.pool.spawn(&t, ox, oy, (0, 0));
                    angle = angle.wrapping_add(0x15);
                }
            }
            if self.phase_frame >= 288 {
                self.phase_frame = 0;
                self.mode = -1;
            }
        }
    }

    /// `reimu_1EF87`: fast random spreads + aimed red cloud stacks.
    fn reimu_m_random_spray(&mut self, env: &mut BossEnv) {
        let (bx, by) = self.reimu_bo();
        if self.reimu_gate(env) == 1 {
            if self.phase_frame % 4 == 0 {
                let angle = (self.rng.and(7) as u8).wrapping_sub(0x44);
                let t = BulletTemplate {
                    spawn_type: 1,
                    group: group::SPREAD,
                    count: SPREAD3_COUNT,
                    delta: SPREAD3_DELTA,
                    speed: (8 * SUBPIXEL) as u8,
                    angle,
                    ..Default::default()
                };
                env.pool.spawn(&t, bx, by, (0, 0));
                if self.phase_frame % 16 == 0 {
                    let (ox, oy) = self.reimu_random_origin();
                    let s = BulletTemplate {
                        spawn_type: 4,
                        group: group::STACK_AIMED,
                        count: 4,
                        delta: SUBPIXEL as u8,
                        speed: (2 * SUBPIXEL) as u8,
                        patnum: PAT_BALL_RED,
                        ..Default::default()
                    };
                    env.pool.spawn(&s, ox, oy, env.player);
                }
            }
            if self.phase_frame >= 192 {
                self.phase_frame = 0;
                self.mode = -1;
            }
        }
    }

    /// `reimu_1F04E`: aimed random-speed pellets, then speeding 32-rings.
    fn reimu_m_aimed_speedup(&mut self, env: &mut BossEnv) {
        let (bx, by) = self.reimu_bo();
        match self.reimu_gate(env) {
            2 => {
                self.angle = iatan2(env.player.1 - self.y, env.player.0 - self.x);
            }
            1 => {
                if self.phase_frame < 96 {
                    if self.phase_frame % 2 == 0 {
                        let t = BulletTemplate {
                            spawn_type: 1,
                            group: group::RANDOM_ANGLE_AND_SPEED,
                            count: 3,
                            speed: (3 * SUBPIXEL + 6) as u8,
                            angle: self.angle,
                            ..Default::default()
                        };
                        env.pool.spawn(&t, bx, by, (0, 0));
                    }
                } else if self.phase_frame <= 128 {
                    if self.phase_frame % 16 == 0 {
                        let t = BulletTemplate {
                            spawn_type: 5,
                            group: group::RING,
                            count: 32,
                            speed: (3 * SUBPIXEL) as u8,
                            angle: self.rng.byte(),
                            patnum: PAT_BALL_BLUE,
                            special: Bsm::Speedup,
                            speed_delta: 1,
                            ..Default::default()
                        };
                        env.pool.spawn(&t, bx, by, (0, 0));
                    }
                } else {
                    self.phase_frame = 0;
                    self.mode = -1;
                }
            }
            _ => {}
        }
    }

    /// `reimu_1F111`: launch moving orbs every `ORB_INTERVAL` frames.
    fn reimu_m_moving_orbs(&mut self, _env: &mut BossEnv) {
        if self.phase_frame == 32 {
            self.sb[5] = 0; // orb_template angle
        }
        if self.phase_frame >= 32 && self.phase_frame % ORB_INTERVAL == 0 {
            self.sb[5] = (self.sb[5] as i32 - self.sb[0]) & 0xFF;
            self.reimu_orb_add_moving(self.sb[5] as u8);
        }
        if self.phase_frame >= 180 {
            self.phase_frame = 0;
            self.mode = -1;
            self.sb[0] = -self.sb[0];
        }
    }

    /// `reimu_1F22A`: alternating wide cloud spreads from random origins.
    fn reimu_m_alt_spread(&mut self, env: &mut BossEnv) {
        match self.reimu_gate(env) {
            2 => {
                self.sb[3] = if self.sb[3] == 0x78 { -0x78 } else { 0x78 };
            }
            1 => {
                if self.phase_frame % 4 == 0 {
                    let speed = (self.rng.and(0x1F) as u8).wrapping_add(SUBPIXEL as u8);
                    let count = (self.rng.and(3) as u8) + 2;
                    let (ox, oy) = self.reimu_random_origin();
                    let base = (0x100i32 - 0x40 + self.sb[3]) as u8;
                    let t = BulletTemplate {
                        spawn_type: 4,
                        group: group::SPREAD,
                        count,
                        delta: 8,
                        speed,
                        angle: base,
                        patnum: PAT_BALL_BLUE,
                        ..Default::default()
                    };
                    env.pool.spawn(&t, ox, oy, (0, 0));
                    let t2 = BulletTemplate { angle: base.wrapping_add(0x80), ..t };
                    env.pool.spawn(&t2, ox, oy, (0, 0));
                }
                if self.phase_frame >= 224 {
                    self.phase_frame = 0;
                    self.mode = -1;
                }
            }
            _ => {}
        }
    }

    /// `reimu_1F2F3`: aimed 5-way cloud-stack fan.
    fn reimu_m_stack_fan(&mut self, env: &mut BossEnv) {
        if self.reimu_gate(env) == 1 {
            if self.phase_frame % 32 == 16 {
                let mut angle: u8 = 0x20;
                for _ in 0..5 {
                    let t = BulletTemplate {
                        spawn_type: 4,
                        group: group::STACK_AIMED,
                        count: STACK_COUNT,
                        delta: 12,
                        speed: (2 * SUBPIXEL) as u8,
                        angle,
                        patnum: PAT_BALL_BLUE,
                        ..Default::default()
                    };
                    env.pool.spawn(&t, self.x, self.y, env.player);
                    angle = angle.wrapping_sub(0x10);
                }
            }
            if self.phase_frame >= 128 {
                self.phase_frame = 0;
                self.mode = -1;
            }
        }
    }

    /// `reimu_1F17C`: the final spell — random-origin 8-way pellet-stack fans.
    fn reimu_m_final(&mut self, env: &mut BossEnv) {
        if self.phase_frame >= 32 && self.phase_frame % 16 == 0 {
            let (ox, oy) = self.reimu_random_origin();
            let mut angle = self.rng.byte();
            for _ in 0..8 {
                let t = BulletTemplate {
                    spawn_type: 1,
                    group: group::STACK,
                    count: 4,
                    delta: 10,
                    speed: (SUBPIXEL + 8) as u8,
                    angle,
                    ..Default::default()
                };
                env.pool.spawn(&t, ox, oy, (0, 0));
                angle = angle.wrapping_add(0x20);
            }
        }
    }
}
