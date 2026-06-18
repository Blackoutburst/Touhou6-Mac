//! Stage 4 rival **Marisa** — `@marisa_update$qv` (th04_main.asm:16535) + the bit
//! engine (`marisa_16C05`/`16C6A`/bit-fire helpers), `flystep_pointreflected`
//! (decompiled in b4m.cpp), and mode subs `marisa_16DFF`..`17813`. The fight when
//! playing as Reimu. HP 6000.
//!
//! Exact: the four **bits** (destructible satellites, hp 220/400/280/450) that
//! spin out to a 64px orbit and act as armour — `marisa_179BC` divides incoming
//! damage by `(bits_alive + 1)`, so the bits must be shot down to hurt Marisa;
//! the point-reflected flight she uses while bitless; the `0xFF` wander-selector
//! that drives the mode cycle (intro-toggle/respawn when bitless, else a random
//! 1-7 bit-pattern); and every mode's danmaku.
//!
//! Approximations: a couple of `boss_statebyte` setup values use Normal-rank
//! constants; bit HP values are the standard set; the `randring2` sequence is the
//! engine-wide PRNG deviation. Bullet special motions (decel/speedup) are exact.

use super::{Boss, BossEnv, Orbit, SUBPIXEL};
use crate::bullet::{group, Bsm, BulletTemplate};
use crate::math::{cos8, iatan2, sin8};

const PAT_BALL_BLUE: u8 = 2;
const PAT_BALL_RED: u8 = 6;
const PAT_STAR: u8 = 7;
const PAT_D_BLUE: u8 = 8;
const PAT_BIT: u8 = 10;
const BIT_HP: [i32; 4] = [220, 400, 280, 450];

impl Boss {
    pub(super) fn marisa_update(&mut self, env: &mut BossEnv) {
        match self.phase {
            0 => {
                self.sb[7] = 2; // byte_25671 (bit angle_speed seed)
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
                    self.phase = 2;
                    self.phase_frame = 0;
                    self.phase_state = 0;
                    self.phase_end_hp = 0;
                    self.mode = 0x0A;
                    self.sb[1] = 0x0A; // last mode
                    self.sb[2] = 0; // bits snapshot
                    self.sb[3] = 1; // intro toggle
                    self.sb[4] = 0; // hp tier
                    self.sb[6] = 0; // flystep tick
                }
            }
            2 => self.marisa_attack(env),
            _ => {
                self.phase_frame += 1;
                if self.phase_frame >= 32 {
                    env.fx.spark(self.x, self.y);
                    self.begin_defeat(env.pool);
                }
            }
        }
        self.marisa_bits_step();
    }

    fn marisa_attack(&mut self, env: &mut BossEnv) {
        match self.mode {
            0 => self.marisa_m_spawn(env),
            1 => self.marisa_m_perp_or_fan(env),
            2 => self.marisa_m_aimed_stars(env),
            3 => self.marisa_m_inout_rings(env),
            4 => self.marisa_m_dense_ring(env),
            5 => self.marisa_m_decel_stars(env),
            6 => self.marisa_m_edge_bullets(env),
            7 => self.marisa_m_mirror(env),
            10 => self.marisa_m_twin_random(env),
            11 => self.marisa_m_rotring(env),
            _ => self.marisa_selector(env),
        }
        if self.phase != 2 {
            return; // selector triggered the defeat
        }
        // marisa_179BC: damage divided by (bits_alive + 1); end at phase_end.
        self.phase_frame += 1;
        let dmg = std::mem::take(&mut self.damage_pending);
        self.hp -= dmg / (self.marisa_bits_alive() + 1);
        if self.hp <= self.phase_end_hp {
            self.phase_state = 1;
            self.phase = 3;
            self.phase_frame = 0;
            return;
        }
        // HP item-drop checkpoints (clear bullets + advance the tier counter).
        self.marisa_checkpoints(env);
    }

    fn marisa_checkpoints(&mut self, env: &mut BossEnv) {
        let crossed = (self.sb[4] == 0 && self.hp <= 4500)
            || (self.sb[4] == 1 && self.hp <= 2500)
            || (self.sb[4] == 2 && self.hp <= 1000);
        if crossed {
            for b in env.pool.bullets.iter_mut() {
                b.active = false;
            }
            self.sb[4] += 1;
        }
    }

    /// `0xFF` wander + mode selector (`marisa_16AE9` + `loc_17B1E`).
    fn marisa_selector(&mut self, env: &mut BossEnv) {
        // Wander within (112..272, 80..144), flipping every 32 frames.
        if self.phase_frame & 0x1F == 1 {
            self.vx = if self.x > 112 * SUBPIXEL && self.x < 272 * SUBPIXEL {
                if self.rng.and(1) != 0 { SUBPIXEL } else { -SUBPIXEL }
            } else if self.x <= 112 * SUBPIXEL {
                2 * SUBPIXEL
            } else {
                -2 * SUBPIXEL
            };
            self.vy = if self.y <= 80 * SUBPIXEL {
                SUBPIXEL
            } else if self.y >= 144 * SUBPIXEL {
                -SUBPIXEL
            } else if self.rng.and(1) != 0 {
                SUBPIXEL
            } else {
                -SUBPIXEL
            };
        }
        self.x += self.vx;
        self.y += self.vy;
        if self.phase_frame < 64 {
            return;
        }
        // Pick the next mode.
        self.phase_state += 1;
        self.sb[6] = 0; // flystep tick
        let alive = self.marisa_bits_alive();
        if self.sb[2] == 0 && alive == 0 {
            self.sb[3] += 1;
            if self.sb[3] >= 2 {
                self.mode = 0;
                self.sb[3] = 0;
            } else {
                self.mode = (self.rng.and(1) as i16) + 0x0A;
            }
        } else {
            let mut m;
            loop {
                m = (self.rng.modulo(7) as i16) + 1;
                if m != self.sb[1] as i16 {
                    break;
                }
            }
            self.mode = m;
            self.sb[1] = m as i32;
            self.sb[2] = alive;
        }
        self.phase_frame = 0;
        if self.phase_state >= 52 {
            self.phase_state = 0;
            env.fx.spark(self.x, self.y);
            self.phase = 3;
            self.phase_frame = 0;
        }
    }

    // === Bit engine =======================================================

    fn marisa_bits_alive(&self) -> i32 {
        self.orbits.iter().filter(|o| (1..=3).contains(&o.flag)).count() as i32
    }

    fn marisa_spawn_bits(&mut self) {
        let base = self.rng.byte();
        let aspeed = self.sb[7] as i8;
        let (bx, by) = (self.x, self.y);
        for (i, o) in self.orbits.iter_mut().enumerate() {
            *o = Orbit {
                flag: 1,
                angle: base.wrapping_add((i as u8).wrapping_mul(64)),
                aspeed,
                cx: bx,
                cy: by,
                ox: 0,
                oy: 0,
                vx: 0,
                vy: 0,
                dist: 0,
                spin_time: 0,
                mspeed: 2 * SUBPIXEL,
                hp: BIT_HP[i],
                patnum: PAT_BIT + i as u8,
            };
        }
    }

    /// `marisa_16C6A` motion half (collision/HP handled in `sim`).
    fn marisa_bits_step(&mut self) {
        let (bx, by) = (self.x, self.y);
        for o in self.orbits.iter_mut() {
            if !(1..=3).contains(&o.flag) {
                continue;
            }
            o.angle = o.angle.wrapping_add(o.aspeed as u8);
            o.dist += o.mspeed;
            o.cx = bx + ((cos8(o.angle) * o.dist) >> 8);
            o.cy = by + ((sin8(o.angle) * o.dist) >> 8);
            if o.flag == 1 && o.dist >= 64 * SUBPIXEL {
                o.flag = 2;
                o.mspeed = 0;
            } else if o.flag == 2 && o.dist >= 4 * SUBPIXEL {
                o.flag = 3;
            }
        }
    }

    /// Fire from every alive bit (`marisa_16DD7` + the two `bit_fire` variants).
    /// `kind` 0 = perpendicular (`16F24`), 1 = template angle (`17061`).
    fn marisa_bits_fire(&mut self, env: &mut BossEnv, template: &BulletTemplate, kind: u8) {
        let alive = self.marisa_bits_alive();
        let firing: Vec<(u8, i32, i32, i8)> = self
            .orbits
            .iter()
            .filter(|o| (1..=3).contains(&o.flag))
            .map(|o| (o.angle, o.cx, o.cy, o.aspeed))
            .collect();
        for (bangle, cx, cy, aspeed) in firing {
            let mut t = *template;
            if kind == 0 {
                if alive <= 2 {
                    t.count = 5;
                }
                let off: u8 = if aspeed >= 0 { 0x40u8.wrapping_neg() } else { 0x40 };
                t.angle = bangle.wrapping_add(off);
            }
            env.pool.spawn(&t, cx, cy, env.player);
        }
    }

    fn marisa_bo(&self) -> (i32, i32) {
        (self.x - 20 * SUBPIXEL, self.y - 8 * SUBPIXEL)
    }

    /// b4m.cpp `marisa_flystep_pointreflected`: fly past (192, 112) then brake.
    fn marisa_flystep(&mut self, duration: i32) -> bool {
        if self.sb[6] == 0 {
            let d = (duration / 2) - 6;
            if d != 0 {
                self.vx = (192 * SUBPIXEL - self.x) / d;
                self.vy = (112 * SUBPIXEL - self.y) / d;
            }
        }
        self.sb[6] += 1;
        if self.sb[6] >= duration - 12 {
            self.vx /= 2;
            self.vy /= 2;
        }
        if self.sb[6] >= duration {
            return true;
        }
        self.x += self.vx;
        self.y += self.vy;
        false
    }

    /// `marisa_16A1A`: telegraph gate — 0 before frame 64, 2 at 64, 1 after.
    fn marisa_gate(&mut self, env: &mut BossEnv) -> i32 {
        let pf = self.phase_frame;
        if pf == 30 {
            let (bx, by) = self.marisa_bo();
            env.fx.gather(bx, by);
            env.fx.circle_shrink(bx, by);
        }
        match pf.cmp(&64) {
            std::cmp::Ordering::Less => 0,
            std::cmp::Ordering::Equal => 2,
            std::cmp::Ordering::Greater => 1,
        }
    }

    fn marisa_end_pattern(&mut self) {
        self.phase_frame = 0;
        self.mode = -1;
    }

    fn marisa_negate_bits(&mut self) {
        for o in self.orbits.iter_mut() {
            o.aspeed = -o.aspeed;
        }
    }

    // === Modes ============================================================

    /// mode 0 (`marisa_16E9D`): telegraph, then spawn the bits.
    fn marisa_m_spawn(&mut self, env: &mut BossEnv) {
        match self.phase_frame {
            0x20 | 0x22 | 0x24 => env.fx.gather(self.x, self.y),
            0x40 => {
                self.marisa_spawn_bits();
                self.sb[7] = -self.sb[7];
            }
            f if f >= 0x60 => self.marisa_end_pattern(),
            _ => {}
        }
    }

    /// mode 0x0A (`marisa_16DFF`): twin random spreads, base angle drifting 0x80→0.
    fn marisa_m_twin_random(&mut self, env: &mut BossEnv) {
        match self.marisa_gate(env) {
            2 => self.angle = 0x80,
            1 => {
                if self.phase_frame % 2 == 0 {
                    let (bx, by) = self.marisa_bo();
                    self.angle = self.angle.wrapping_sub(8);
                    for side in [-6, 12] {
                        let count = (self.rng.and(3) as u8) + 1;
                        let speed = (self.rng.and(0x1F) as u8).wrapping_add(2 * SUBPIXEL as u8);
                        let t = BulletTemplate {
                            spawn_type: 4,
                            group: group::SPREAD,
                            count,
                            delta: 8,
                            speed,
                            angle: self.angle,
                            patnum: PAT_BALL_BLUE,
                            ..Default::default()
                        };
                        env.pool.spawn(&t, bx + side * SUBPIXEL, by, (0, 0));
                    }
                }
                if self.angle == 0 {
                    self.marisa_end_pattern();
                }
            }
            _ => {}
        }
    }

    /// mode 0x0B (`marisa_17813`): a rotating 32-ring of pellets.
    fn marisa_m_rotring(&mut self, env: &mut BossEnv) {
        match self.marisa_gate(env) {
            2 => self.sb[5] = if self.rng.and(1) != 0 { -1 } else { 1 },
            1 => {
                if self.phase_frame % 8 == 0 {
                    let (bx, by) = self.marisa_bo();
                    let t = BulletTemplate {
                        spawn_type: 1,
                        group: group::RING,
                        count: 32,
                        speed: (3 * SUBPIXEL + 8) as u8,
                        angle: self.angle,
                        ..Default::default()
                    };
                    env.pool.spawn(&t, bx, by, (0, 0));
                    self.angle = self.angle.wrapping_add(self.sb[5] as u8);
                }
                if self.phase_frame >= 128 {
                    self.marisa_end_pattern();
                }
            }
            _ => {}
        }
    }

    /// mode 1 (`marisa_16F61`): bits fire perpendicular spreads; when bitless,
    /// Marisa flies and rakes a 4-way blue spread.
    fn marisa_m_perp_or_fan(&mut self, env: &mut BossEnv) {
        match self.marisa_gate(env) {
            2 => self.sb[5] = self.phase_frame, // last_frame_with_bits
            1 => {
                if self.phase_frame % 4 == 0 {
                    if self.marisa_bits_alive() > 0 {
                        let t = BulletTemplate {
                            spawn_type: 1,
                            group: group::SPREAD,
                            count: 3,
                            delta: 8,
                            speed: (3 * SUBPIXEL + 8) as u8,
                            ..Default::default()
                        };
                        self.marisa_bits_fire(env, &t, 0);
                        self.sb[5] = self.phase_frame;
                    } else {
                        let done = self.marisa_flystep(160 - self.sb[5]);
                        let _ = done;
                        let (bx, by) = self.marisa_bo();
                        let mut a = self.angle.wrapping_add(6);
                        for _ in 0..4 {
                            let t = BulletTemplate {
                                spawn_type: 2,
                                group: group::SPREAD,
                                count: 3,
                                delta: 6,
                                speed: (3 * SUBPIXEL + 4) as u8,
                                angle: a,
                                patnum: PAT_D_BLUE,
                                ..Default::default()
                            };
                            env.pool.spawn(&t, bx, by, (0, 0));
                            a = a.wrapping_add(0x40);
                        }
                        self.angle = a;
                    }
                }
                if self.phase_frame >= 160 {
                    self.marisa_negate_bits();
                    self.marisa_end_pattern();
                }
            }
            _ => {}
        }
    }

    /// mode 2 (`marisa_17079`): bits fire aimed stars; bitless → aimed star spread.
    fn marisa_m_aimed_stars(&mut self, env: &mut BossEnv) {
        match self.marisa_gate(env) {
            2 => {
                for o in self.orbits.iter_mut() {
                    o.aspeed = o.aspeed.wrapping_mul(2);
                }
                self.sb[5] = self.phase_frame;
                self.sb[3] = SUBPIXEL; // reuse: bit-fire speed ramp
            }
            1 => {
                if self.phase_frame % 4 == 0 {
                    self.angle = iatan2(env.player.1 - self.y, env.player.0 - self.x);
                    if self.marisa_bits_alive() > 0 {
                        let t = BulletTemplate {
                            spawn_type: 1,
                            group: group::SINGLE,
                            count: 1,
                            speed: self.sb[3] as u8,
                            angle: self.angle,
                            ..Default::default()
                        };
                        self.marisa_bits_fire(env, &t, 1);
                        self.sb[5] = self.phase_frame;
                    } else {
                        self.marisa_flystep(160 - self.sb[5]);
                        let (bx, by) = self.marisa_bo();
                        let t = BulletTemplate {
                            spawn_type: 2,
                            group: group::SPREAD_AIMED,
                            count: 3,
                            delta: 0x0C,
                            speed: self.sb[3] as u8,
                            patnum: PAT_STAR,
                            ..Default::default()
                        };
                        env.pool.spawn(&t, bx, by, env.player);
                    }
                    self.sb[3] += 4;
                }
                if self.phase_frame >= 160 {
                    for o in self.orbits.iter_mut() {
                        o.aspeed /= 2;
                    }
                    self.marisa_end_pattern();
                }
            }
            _ => {}
        }
    }

    /// mode 3 (`marisa_1717D`): bits move out/in while firing aimed rings;
    /// bitless → twin random spreads.
    fn marisa_m_inout_rings(&mut self, env: &mut BossEnv) {
        match self.marisa_gate(env) {
            2 => self.sb[5] = 0, // subpattern_num
            1 => {
                if self.marisa_bits_alive() > 0 {
                    let pf = self.phase_frame;
                    if pf <= 192 {
                        for o in self.orbits.iter_mut().filter(|o| (1..=3).contains(&o.flag)) {
                            o.dist += 0x18;
                        }
                    } else if pf <= 256 {
                        if pf % 4 == 0 {
                            if pf % 32 == 0 {
                                let t = BulletTemplate {
                                    spawn_type: 1,
                                    group: group::RING_AIMED,
                                    count: 16,
                                    speed: (2 * SUBPIXEL) as u8,
                                    ..Default::default()
                                };
                                env.pool.spawn(&t, self.x, self.y, env.player);
                            }
                            let t = BulletTemplate {
                                spawn_type: 1,
                                group: group::SPREAD,
                                count: 3,
                                delta: 6,
                                speed: (4 * SUBPIXEL) as u8,
                                ..Default::default()
                            };
                            self.marisa_bits_fire(env, &t, 0);
                        }
                    } else if pf <= 384 {
                        for o in self.orbits.iter_mut().filter(|o| (1..=3).contains(&o.flag)) {
                            o.dist -= 0x18;
                        }
                    } else {
                        for (i, o) in self.orbits.iter_mut().enumerate() {
                            if i % 2 == 0 {
                                o.aspeed = -o.aspeed;
                            }
                        }
                        self.marisa_end_pattern();
                    }
                } else {
                    self.marisa_flystep(96);
                    if self.phase_frame % 2 == 0 {
                        let (bx, by) = self.marisa_bo();
                        let dir: u8 = if self.sb[5] & 1 != 0 { 0xF8 } else { 8 };
                        self.angle = self.angle.wrapping_add(dir);
                        for side in [-6, 12] {
                            let count = (self.rng.and(3) as u8) + 1;
                            let speed = (self.rng.and(0x1F) as u8).wrapping_add(SUBPIXEL as u8);
                            let t = BulletTemplate {
                                spawn_type: 4,
                                group: group::SPREAD,
                                count,
                                delta: 8,
                                speed,
                                angle: self.angle,
                                patnum: PAT_BALL_BLUE,
                                ..Default::default()
                            };
                            env.pool.spawn(&t, bx + side * SUBPIXEL, by, (0, 0));
                        }
                        if self.angle == 0 || self.angle >= 0x80 {
                            self.sb[5] += 1;
                            if self.sb[5] >= 4 {
                                self.marisa_end_pattern();
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// mode 4 (`marisa_17335`): bits fire dense aimed rings; bitless → scattered
    /// red aimed cloud bullets.
    fn marisa_m_dense_ring(&mut self, env: &mut BossEnv) {
        match self.marisa_gate(env) {
            2 => {}
            1 => {
                if self.marisa_bits_alive() > 0 {
                    if self.phase_frame % 32 == 0 {
                        let mut count = 24 - (self.marisa_bits_alive() as u8) * 2;
                        if self.sb[4] == 2 {
                            count = 28;
                        }
                        let t = BulletTemplate {
                            spawn_type: 5,
                            group: group::RING_AIMED,
                            count,
                            speed: (3 * SUBPIXEL + 2) as u8,
                            patnum: PAT_BALL_BLUE,
                            ..Default::default()
                        };
                        self.marisa_bits_fire(env, &t, 1);
                    }
                    if self.phase_frame >= 160 {
                        for (i, o) in self.orbits.iter_mut().enumerate() {
                            if i % 2 == 0 {
                                o.aspeed = -o.aspeed;
                            }
                        }
                        self.marisa_end_pattern();
                    }
                } else if self.marisa_flystep(64) {
                    self.marisa_end_pattern();
                } else if self.phase_frame % 16 == 0 {
                    for _ in 0..0x20 {
                        let ox = self.x - 52 * SUBPIXEL + self.rng.modulo(64 * SUBPIXEL);
                        let oy = self.y - 40 * SUBPIXEL + self.rng.modulo(64 * SUBPIXEL);
                        let speed = (self.rng.modulo(6 * SUBPIXEL) + SUBPIXEL) as u8;
                        let t = BulletTemplate {
                            spawn_type: 4,
                            group: group::SINGLE_AIMED,
                            count: 1,
                            speed,
                            patnum: PAT_BALL_RED,
                            ..Default::default()
                        };
                        env.pool.spawn(&t, ox, oy, env.player);
                    }
                }
            }
            _ => {}
        }
    }

    /// mode 5 (`marisa_17491`): bits fire decelerate-to-angle star volleys;
    /// bitless → stacked blue clouds.
    fn marisa_m_decel_stars(&mut self, env: &mut BossEnv) {
        match self.marisa_gate(env) {
            2 => self.sb[5] = 0, // bitless_started
            1 => {
                if self.marisa_bits_alive() > 0 {
                    if self.phase_frame % 4 == 0 {
                        // Volley target sweeps across phase windows.
                        let (target, base): (u8, u8) = match self.phase_frame {
                            f if f <= 96 => (0x40, 0x10),
                            f if f <= 128 => (0x30, 0x90),
                            f if f <= 160 => (0x70, 0x10),
                            f if f <= 192 => (0x10, 0x10),
                            _ => (0x40, 0x10),
                        };
                        let mut a = base;
                        for _ in 0..4 {
                            let t = BulletTemplate {
                                spawn_type: 2,
                                group: group::SINGLE,
                                count: 1,
                                speed: (5 * SUBPIXEL + 12) as u8,
                                angle: a,
                                patnum: PAT_STAR,
                                special: Bsm::DecelToAngle,
                                turn_arg: target,
                                ..Default::default()
                            };
                            self.marisa_bits_fire(env, &t, 1);
                            a = a.wrapping_add(0x20);
                        }
                    }
                    if self.phase_frame >= 224 {
                        self.marisa_end_pattern();
                    }
                } else {
                    self.marisa_flystep(128);
                    if self.sb[5] == 0 {
                        self.angle = 0x80u8.wrapping_sub(self.rng.and(0x1F) as u8);
                        self.sb[5] = 1;
                    }
                    if self.phase_frame % 8 == 0 {
                        let (bx, by) = self.marisa_bo();
                        let t = BulletTemplate {
                            spawn_type: 4,
                            group: group::STACK,
                            count: 16,
                            delta: 5,
                            speed: SUBPIXEL as u8,
                            angle: self.angle,
                            patnum: PAT_BALL_BLUE,
                            ..Default::default()
                        };
                        env.pool.spawn(&t, bx, by, (0, 0));
                        self.angle = self.angle.wrapping_sub(8);
                    }
                    if self.phase_frame >= 256 {
                        self.marisa_end_pattern();
                    }
                }
            }
            _ => {}
        }
    }

    /// mode 6 (`marisa_1769E`): bits fire aimed stars; bitless → wall bullets.
    fn marisa_m_edge_bullets(&mut self, env: &mut BossEnv) {
        match self.marisa_gate(env) {
            2 => {
                self.angle = 0x80u8.wrapping_sub(0x40); // -0x40
                self.sb[5] = 0;
            }
            1 => {
                if self.marisa_bits_alive() > 0 {
                    if self.phase_frame <= 128 {
                        if env.frame % 4 == 0 {
                            let t = BulletTemplate {
                                spawn_type: 2,
                                group: group::SINGLE,
                                count: 1,
                                speed: (6 * SUBPIXEL) as u8,
                                angle: self.angle,
                                patnum: PAT_STAR,
                                ..Default::default()
                            };
                            self.marisa_bits_fire(env, &t, 1);
                        }
                    } else if self.phase_frame <= 192 {
                        if env.frame % 4 == 0 {
                            // Bullets stream in from the left, right and top edges.
                            let oy1 = self.rng.modulo(192 * SUBPIXEL);
                            let a1 = 0x30u8.wrapping_sub(self.rng.and(0x1F) as u8);
                            self.marisa_edge_bullet(env, 0, oy1, a1);
                            let oy2 = self.rng.modulo(192 * SUBPIXEL);
                            let a2 = (self.rng.and(0x1F) as u8).wrapping_add(0x50);
                            self.marisa_edge_bullet(env, 384 * SUBPIXEL, oy2, a2);
                            let ox = self.rng.modulo(384 * SUBPIXEL);
                            let a3 = (self.rng.and(0x1F) as u8).wrapping_add(0x30);
                            self.marisa_edge_bullet(env, ox, 0, a3);
                        }
                    } else if self.phase_frame >= 256 {
                        self.marisa_end_pattern();
                    }
                } else {
                    self.marisa_flystep(160);
                    if self.sb[5] == 0 {
                        self.angle = self.rng.and(0x1F) as u8;
                        self.sb[5] = 1;
                    }
                    if self.phase_frame % 8 == 0 {
                        let (bx, by) = self.marisa_bo();
                        let t = BulletTemplate {
                            spawn_type: 4,
                            group: group::STACK,
                            count: 16,
                            delta: 5,
                            speed: SUBPIXEL as u8,
                            angle: self.angle,
                            patnum: PAT_BALL_BLUE,
                            ..Default::default()
                        };
                        env.pool.spawn(&t, bx, by, (0, 0));
                        self.angle = self.angle.wrapping_add(8);
                    }
                    if self.phase_frame >= 192 {
                        self.marisa_end_pattern();
                    }
                }
            }
            _ => {}
        }
    }

    fn marisa_edge_bullet(&mut self, env: &mut BossEnv, ox: i32, oy: i32, angle: u8) {
        let speed = ((self.rng.and(0x1F) as u8).wrapping_add(SUBPIXEL as u8)) as u8;
        let t = BulletTemplate {
            spawn_type: 4,
            group: group::SINGLE,
            count: 1,
            speed,
            angle,
            patnum: PAT_BALL_BLUE,
            ..Default::default()
        };
        env.pool.spawn(&t, ox, oy, (0, 0));
    }

    /// mode 7 (`marisa_1788E`): bits fire spinning pellets + accelerating clouds;
    /// bitless → aimed cloud stack.
    fn marisa_m_mirror(&mut self, env: &mut BossEnv) {
        match self.marisa_gate(env) {
            2 => {
                self.sb[3] = 2 * SUBPIXEL; // spread_speed
                self.sb[5] = self.rng.and(1) as i32; // angle mirror
            }
            1 => {
                if self.marisa_bits_alive() > 0 {
                    if env.frame % 4 == 0 {
                        let mut a = (env.frame as u8).wrapping_shl(3);
                        if self.sb[5] != 0 {
                            a = (a as i8).wrapping_neg() as u8;
                        }
                        let t = BulletTemplate {
                            spawn_type: 1,
                            group: group::SINGLE,
                            count: 1,
                            speed: (2 * SUBPIXEL) as u8,
                            angle: a,
                            ..Default::default()
                        };
                        self.marisa_bits_fire(env, &t, 1);
                    }
                    if env.frame % 8 == 0 {
                        let t = BulletTemplate {
                            spawn_type: 4,
                            group: group::SINGLE,
                            count: 1,
                            speed: self.sb[3] as u8,
                            angle: 0,
                            patnum: PAT_BALL_BLUE,
                            ..Default::default()
                        };
                        self.marisa_bits_fire(env, &t, 0);
                        self.sb[3] += 2;
                    }
                    if self.phase_frame >= 160 {
                        self.marisa_negate_bits();
                        self.marisa_end_pattern();
                    }
                } else if self.marisa_flystep(72) {
                    self.marisa_end_pattern();
                } else if self.phase_frame % 8 == 0 {
                    let (bx, by) = self.marisa_bo();
                    let t = BulletTemplate {
                        spawn_type: 4,
                        group: group::STACK_AIMED,
                        count: 16,
                        delta: 5,
                        speed: SUBPIXEL as u8,
                        patnum: PAT_BALL_BLUE,
                        ..Default::default()
                    };
                    env.pool.spawn(&t, bx, by, env.player);
                }
            }
            _ => {}
        }
    }
}
