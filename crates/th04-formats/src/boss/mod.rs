//! TH04 bosses, ported from ReC98's `th04_main.asm` boss update functions.
//! The stage 1-5 bosses are *not* decompiled to C++ in ReC98 — only their
//! position-independent x86 disassembly exists — so each is reversed straight
//! from the asm. This module is the shared boss engine (modelled on
//! `th04/main/boss/boss.cpp`: `boss_stuff_t`, `boss_phase_next`,
//! `boss_hittest_shots`); each boss lives in its own submodule and implements a
//! per-frame `*_update` on [`Boss`].
//!
//! All patterns target **Normal** rank (on Normal `bullet_template_tune` is a
//! no-op for counts, so the literal template counts are exact).
//!
//! Deliberate, feel-preserving deviations from the originals:
//!  * Cosmetic effects (gather/circle/spark telegraphs, palette tints, screen
//!    shake, explosions) carry no collision; they are surfaced as non-damaging
//!    [`crate::effects`] markers a renderer can draw, not skipped silently.
//!  * The exact `randring2` ring sequence (shared with every other RNG consumer
//!    in the frame) is infeasible to reproduce; a local PRNG ([`BossRng`]) drives
//!    pattern selection and random angles/positions instead.
//!
//! Positions/velocities are subpixels (16/px); angles are 256-direction bytes
//! (0 = +x, 64 = +y). See [`crate::math`] and [`crate::bullet`].

use crate::bullet::BulletPool;
use crate::effects::EffectPool;

mod elly;
mod kurumi;
mod marisa;
mod orange;
mod reimu;
mod yuuka;
mod yuuka6;

const SUBPIXEL: i32 = 16;
/// Playfield bounds (subpixels) used by sub-entity edge tests.
pub(crate) const PLAYFIELD_W: i32 = 384;
pub(crate) const PLAYFIELD_H: i32 = 368;
/// Boss hitbox half-extent (`BOSS_HITBOX_DEFAULT` = BOSS_W/2 − BOSS_W/8 = 24px).
pub const BOSS_HIT: i32 = 24 * SUBPIXEL;
/// Frames the defeat (explode) sequence runs before the stage may advance.
const BOSS_DEFEAT_FRAMES: u32 = 120;

/// Which boss this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BossKind {
    /// Stage 1 — Orange (`@orange_update$qv`).
    Orange,
    /// Stage 2 — Kurumi (`@kurumi_update$qv`).
    Kurumi,
    /// Stage 3 — Elly (`@elly_update$qv`).
    Elly,
    /// Stage 4 rival — Reimu (`@reimu_update$qv`), fought when playing Marisa.
    Reimu,
    /// Stage 4 rival — Marisa (`@marisa_update$qv`), fought when playing Reimu.
    Marisa,
    /// Stage 5 — Yuuka (`@yuuka5_update$qv`).
    Yuuka,
    /// Stage 6 (final) — Yuuka (`@yuuka6_update$qv`).
    Yuuka6,
}

impl BossKind {
    /// The boss's display name (for the appears-card / HUD).
    pub fn name(self) -> &'static str {
        match self {
            BossKind::Orange => "ORANGE",
            BossKind::Kurumi => "KURUMI",
            BossKind::Elly => "ELLY",
            BossKind::Reimu => "REIMU HAKUREI",
            BossKind::Marisa => "MARISA KIRISAME",
            BossKind::Yuuka | BossKind::Yuuka6 => "YUUKA KAZAMI",
        }
    }

    /// The boss that ends stage `stage` (0-based: `ST00` = 0 = stage 1).
    /// Stage 4 (index 3) is the rival fight, decided by the player character:
    /// Reimu's player faces Marisa and vice-versa. Stages with no roster boss
    /// (e.g. `ST06`/Extra) return `None`.
    pub fn for_stage(stage: usize, playing_marisa: bool) -> Option<BossKind> {
        Some(match stage {
            0 => BossKind::Orange,
            1 => BossKind::Kurumi,
            2 => BossKind::Elly,
            3 => {
                if playing_marisa {
                    BossKind::Reimu
                } else {
                    BossKind::Marisa
                }
            }
            4 => BossKind::Yuuka,
            5 => BossKind::Yuuka6,
            _ => return None,
        })
    }
}

/// Per-frame context handed to a boss update: the bullet pool, the telegraph
/// pool, the player position (for aimed fire) and the global stage frame (for
/// the `stage_frame_mod{2,4,8}` cadences).
pub struct BossEnv<'a> {
    pub pool: &'a mut BulletPool,
    pub fx: &'a mut EffectPool,
    pub player: (i32, i32),
    pub frame: u16,
}

/// Small deterministic PRNG standing in for TH04's `randring2` (see module
/// docs). Returns selection/angle bytes; not the original sequence.
pub(crate) struct BossRng {
    state: u32,
}
impl BossRng {
    fn new(seed: u32) -> Self {
        Self { state: seed }
    }
    fn next(&mut self) -> u32 {
        self.state = self.state.wrapping_mul(1103515245).wrapping_add(12345);
        self.state >> 16
    }
    /// `randring2_next16` — a full angle byte.
    pub(crate) fn byte(&mut self) -> u8 {
        self.next() as u8
    }
    /// `randring2_next16_and(mask)`.
    pub(crate) fn and(&mut self, mask: u32) -> u32 {
        self.next() & mask
    }
    /// `randring2_next16_mod(n)`.
    pub(crate) fn modulo(&mut self, n: i32) -> i32 {
        (self.next() % n.max(1) as u32) as i32
    }
}

/// A Kurumi spawn-ray: a line that grows out from the boss along a fixed
/// direction, fires a burst when its tip leaves the field, then retracts.
#[derive(Clone, Copy, Default)]
pub struct Ray {
    /// 0 = free, 1 = growing, 2 = shrinking.
    pub flag: u8,
    pub ox: i32,
    pub oy: i32,
    pub tx: i32,
    pub ty: i32,
    pub vx: i32,
    pub vy: i32,
}

/// A boss-controlled satellite (Reimu's orbs / Marisa's bits): spins out from
/// an origin then either keeps orbiting or flies free.
#[derive(Clone, Copy, Default)]
pub struct Orbit {
    /// 0 = free, 1 = spinning out, 2 = moving/orbiting.
    pub flag: u8,
    pub angle: u8,
    pub aspeed: i8,
    pub cx: i32,
    pub cy: i32,
    /// Spin-out origin (the boss position at spawn).
    pub ox: i32,
    pub oy: i32,
    pub vx: i32,
    pub vy: i32,
    pub dist: i32,
    pub spin_time: i32,
    /// Linear speed once moving (subpixels).
    pub mspeed: i32,
    pub hp: i32,
    pub patnum: u8,
}

pub struct Boss {
    kind: BossKind,
    // --- boss_stuff_t ---
    /// `boss_pos.cur` (subpixels).
    pub x: i32,
    pub y: i32,
    /// `boss_pos.velocity`.
    vx: i32,
    vy: i32,
    pub hp: i32,
    /// HP-bar maximum (for the HUD).
    pub max_hp: i32,
    pub sprite: u8,
    pub phase: u8,
    phase_frame: i32,
    mode: i16,
    /// `boss_angle` — shared movement/bullet angle.
    angle: u8,
    /// `boss_phase_state` — a counter / per-pattern persistent value.
    phase_state: i16,
    phase_end_hp: i32,
    // statebyte[14]/[15] in the original.
    pattern_num_prev: i16,
    patterns_done: i16,
    /// `boss_statebyte` + per-boss scratch state the per-boss code uses freely.
    sb: [i32; 8],
    // --- sub-entities (only some bosses use these) ---
    /// Kurumi spawn-rays (visible; emit bullets at the field edge).
    pub rays: Vec<Ray>,
    /// Reimu orbs / Marisa bits.
    pub orbits: Vec<Orbit>,
    // --- engine bookkeeping ---
    pub defeated: bool,
    defeat_frame: u32,
    /// Damage accrued from player shots since the last `hittest`.
    damage_pending: i32,
    rng: BossRng,
}

impl Boss {
    fn new(kind: BossKind, hp: i32, x: i32, y: i32, seed: u32) -> Self {
        Boss {
            kind,
            x,
            y,
            vx: 0,
            vy: 0,
            hp,
            max_hp: hp,
            sprite: 0,
            phase: 0,
            phase_frame: 0,
            mode: 0,
            angle: 0,
            phase_state: 0,
            phase_end_hp: hp,
            pattern_num_prev: -1,
            patterns_done: 0,
            sb: [0; 8],
            rays: Vec::new(),
            orbits: Vec::new(),
            defeated: false,
            defeat_frame: 0,
            damage_pending: 0,
            rng: BossRng::new(seed),
        }
    }

    /// Stage 1 boss Orange (HP 3050, centre-top).
    pub fn orange() -> Self {
        Self::new(BossKind::Orange, 3050, 192 * SUBPIXEL, 80 * SUBPIXEL, 0x1234_5678)
    }

    /// Stage 2 boss Kurumi (HP 4800, centre-top). Phase 0 sets the thresholds.
    pub fn kurumi() -> Self {
        let mut b = Self::new(BossKind::Kurumi, 4800, 192 * SUBPIXEL, 64 * SUBPIXEL, 0x0BAD_F00D);
        b.rays = vec![Ray::default(); 6];
        b
    }

    /// Stage 3 boss Elly (HP 6000, centre-top).
    pub fn elly() -> Self {
        Self::new(BossKind::Elly, 6000, 192 * SUBPIXEL, 96 * SUBPIXEL, 0x00C0_FFEE)
    }

    /// Stage 4 rival Reimu (HP 9100) — the fight when playing as Marisa.
    pub fn reimu() -> Self {
        let mut b = Self::new(BossKind::Reimu, 9100, 192 * SUBPIXEL, 96 * SUBPIXEL, 0x5EED_1234);
        b.orbits = vec![Orbit::default(); 8];
        b
    }

    /// Stage 4 rival Marisa (HP 6000) — the fight when playing as Reimu.
    pub fn marisa() -> Self {
        let mut b = Self::new(BossKind::Marisa, 6000, 192 * SUBPIXEL, 96 * SUBPIXEL, 0x5EED_5678);
        b.orbits = vec![Orbit::default(); 4];
        b
    }

    /// Stage 5 boss Yuuka (HP 9000, centre-top).
    pub fn yuuka() -> Self {
        Self::new(BossKind::Yuuka, 9000, 192 * SUBPIXEL, 80 * SUBPIXEL, 0x1234_ABCD)
    }

    /// Stage 6 (final) boss Yuuka (HP 13300). Phase 1 sets the thresholds.
    pub fn yuuka6() -> Self {
        let mut b = Self::new(BossKind::Yuuka6, 13300, 192 * SUBPIXEL, 80 * SUBPIXEL, 0xF1A1_6660);
        b.orbits = vec![Orbit::default(); 24]; // chasecross pool
        b
    }

    /// Construct the boss for a [`BossKind`].
    pub fn from_kind(kind: BossKind) -> Self {
        match kind {
            BossKind::Orange => Self::orange(),
            BossKind::Kurumi => Self::kurumi(),
            BossKind::Elly => Self::elly(),
            BossKind::Reimu => Self::reimu(),
            BossKind::Marisa => Self::marisa(),
            BossKind::Yuuka => Self::yuuka(),
            BossKind::Yuuka6 => Self::yuuka6(),
        }
    }

    /// Which boss this is (for sprite/HUD lookup).
    pub fn kind(&self) -> BossKind {
        self.kind
    }

    /// True once the defeat animation has finished (stage may advance).
    pub fn done(&self) -> bool {
        self.defeated && self.defeat_frame >= BOSS_DEFEAT_FRAMES
    }

    /// Defeat-animation progress (frames since defeat began); pair with
    /// [`Boss::DEFEAT_FRAMES`] for the 0..1 fraction.
    pub fn defeat_frame(&self) -> u32 {
        self.defeat_frame
    }
    /// Total frames the defeat (explosion) sequence runs.
    pub const DEFEAT_FRAMES: u32 = BOSS_DEFEAT_FRAMES;

    /// Queue player-shot (or bomb) damage; consumed at the boss's hittest point.
    pub fn damage(&mut self, dmg: i32) {
        if !self.defeated {
            self.damage_pending += dmg;
        }
    }

    /// `PlayfieldMotion::update` — step by `velocity`, return the new x.
    fn move_step(&mut self) -> i32 {
        self.x += self.vx;
        self.y += self.vy;
        self.x
    }

    /// `boss_hittest_shots` (TH04): increments `phase_frame`, applies queued
    /// damage, returns true once HP has dropped to the phase threshold.
    fn hittest(&mut self) -> bool {
        self.phase_frame += 1;
        self.hp -= std::mem::take(&mut self.damage_pending);
        self.hp <= self.phase_end_hp
    }

    /// `boss_hittest_shots_invincible` — bumps `phase_frame`, drops the damage.
    fn hittest_invincible(&mut self) {
        self.phase_frame += 1;
        self.damage_pending = 0;
    }

    /// `boss_hittest_shots_damage` (phases that bump `phase_frame` themselves).
    fn hittest_damage(&mut self) {
        self.hp -= std::mem::take(&mut self.damage_pending);
    }

    /// `boss_phase_next`: advance phase, clear the screen, top HP up to the old
    /// threshold and arm the next one.
    fn phase_next(&mut self, next_end_hp: i32, pool: &mut BulletPool) {
        for b in pool.bullets.iter_mut() {
            b.active = false;
        }
        self.phase += 1;
        self.phase_frame = 0;
        self.mode = 0;
        self.patterns_done = 0;
        self.phase_state = 0;
        self.hp = self.phase_end_hp;
        self.phase_end_hp = next_end_hp;
    }

    /// Enter the defeat (explosion) sequence: invulnerable, no more attacks.
    fn begin_defeat(&mut self, pool: &mut BulletPool) {
        self.defeated = true;
        self.defeat_frame = 0;
        for b in pool.bullets.iter_mut() {
            b.active = false;
        }
    }

    /// Advance one frame: movement + attacks (fires into `env.pool`).
    pub fn update(&mut self, env: &mut BossEnv) {
        if self.defeated {
            self.defeat_frame += 1;
            return;
        }
        match self.kind {
            BossKind::Orange => self.orange_update(env),
            BossKind::Kurumi => self.kurumi_update(env),
            BossKind::Elly => self.elly_update(env),
            BossKind::Reimu => self.reimu_update(env),
            BossKind::Marisa => self.marisa_update(env),
            BossKind::Yuuka => self.yuuka_update(env),
            BossKind::Yuuka6 => self.yuuka6_update(env),
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
                    let t = crate::bullet::BulletTemplate {
                        group: crate::bullet::group::SINGLE,
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
    use crate::bullet::BulletPool;
    use crate::effects::EffectPool;

    fn env<'a>(
        pool: &'a mut BulletPool,
        fx: &'a mut EffectPool,
        player: (i32, i32),
        frame: u16,
    ) -> BossEnv<'a> {
        BossEnv { pool, fx, player, frame }
    }

    #[test]
    fn midboss_entrance_then_defeat() {
        let mut m = Midboss::new(192 * 16);
        let mut pool = BulletPool::new();
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
    fn orange_runs_through_phases_and_dies() {
        let mut b = Boss::orange();
        let mut pool = BulletPool::new();
        let mut fx = EffectPool::new();
        let player = (192 * 16, 400 * 16);
        let mut frame: u16 = 0;
        for _ in 0..352 {
            b.update(&mut env(&mut pool, &mut fx, player, frame));
            frame = frame.wrapping_add(1);
        }
        assert!(b.phase >= 1, "should leave the intro charge");
        for _ in 0..6000 {
            if b.done() {
                break;
            }
            b.damage(40);
            b.update(&mut env(&mut pool, &mut fx, player, frame));
            frame = frame.wrapping_add(1);
        }
        assert!(b.defeated, "boss should be defeated");
        assert!(b.done(), "defeat sequence should finish");
    }

    #[test]
    fn orange_phase1_fires_three_rings() {
        let mut b = Boss::orange();
        let mut pool = BulletPool::new();
        let mut fx = EffectPool::new();
        b.phase = 1;
        b.phase_frame = 31;
        b.update(&mut env(&mut pool, &mut fx, (0, 0), 0));
        assert_eq!(pool.active_count(), 48);
        assert_eq!(b.phase, 2, "should enter the barrage phase");
    }

    fn run_until_dead(mut b: Boss, max: u32) -> Boss {
        let mut pool = BulletPool::new();
        let mut fx = EffectPool::new();
        let player = (192 * 16, 400 * 16);
        let mut frame: u16 = 0;
        for _ in 0..max {
            if b.done() {
                break;
            }
            b.damage(40);
            b.update(&mut env(&mut pool, &mut fx, player, frame));
            frame = frame.wrapping_add(1);
        }
        b
    }

    #[test]
    fn elly_runs_through_tiers_and_dies() {
        let b = run_until_dead(Boss::elly(), 12000);
        assert!(b.defeated, "elly should be defeated");
        assert!(b.done(), "defeat sequence should finish");
    }

    #[test]
    fn rivals_run_through_tiers_and_die() {
        for (name, b) in [("reimu", Boss::reimu()), ("marisa", Boss::marisa())] {
            let b = run_until_dead(b, 16000);
            assert!(b.defeated, "{name} should be defeated");
            assert!(b.done(), "{name} defeat sequence should finish");
        }
    }

    #[test]
    fn yuuka_reaches_master_spark_and_dies() {
        let b = run_until_dead(Boss::yuuka(), 16000);
        assert!(b.defeated, "yuuka should be defeated");
        assert!(b.done(), "defeat sequence should finish");
    }

    #[test]
    fn yuuka6_runs_through_all_phases_and_dies() {
        let b = run_until_dead(Boss::yuuka6(), 30000);
        assert!(b.defeated, "stage-6 yuuka should be defeated");
        assert!(b.done(), "defeat sequence should finish");
    }

    #[test]
    fn kurumi_runs_through_phases_and_dies() {
        let mut b = Boss::kurumi();
        let mut pool = BulletPool::new();
        let mut fx = EffectPool::new();
        let player = (192 * 16, 400 * 16);
        let mut frame: u16 = 0;
        // Survive the invincible intro, then pour in damage.
        for _ in 0..8000 {
            if b.done() {
                break;
            }
            b.damage(40);
            b.update(&mut env(&mut pool, &mut fx, player, frame));
            frame = frame.wrapping_add(1);
        }
        assert!(b.defeated, "kurumi should be defeated");
        assert!(b.done(), "defeat sequence should finish");
    }
}
