//! Headless stage simulation: ties the whole gameplay loop together — the
//! `.STD` spawn timeline, the enemy script VM, the bullet system and the
//! player, plus collision and scoring. No rendering; this is the deterministic
//! game model that a renderer (and the WASM build) will drive and draw.
//!
//! Collision is approximate (axis-aligned boxes) pending the exact TH04
//! hitboxes: player shots damage enemies (kill → score), and enemy bullets that
//! reach the player count as hits. Positions are subpixels (16/px).

use crate::boss::{Boss, BossEnv, BossKind, Midboss, BOSS_HIT};
use crate::bullet::BulletPool;
use crate::effects::EffectPool;
use crate::enemy_vm::Enemy;
use crate::player::{Input, Player};
use crate::stage::{Std, TimelineFrame};

// Midboss-1 hitbox half-extent (RE: 24×16) and activation frame (placeholder
// until the per-stage value is located).
const MIDBOSS_HIT: i32 = 24 * SUBPIXEL;
const MIDBOSS_FRAME: u16 = 2400;
const PLAYFIELD_W: i32 = 384;
const PLAYFIELD_H: i32 = 368;
/// ReC98 ENEMY_POS_RANDOM (999.0 px): a spawn coordinate of this value means
/// "pick a random position on that axis" (randring2_next16_mod). Stored as a
/// subpixel here.
const ENEMY_POS_RANDOM: i32 = 999 * SUBPIXEL;

const SUBPIXEL: i32 = 16;
const SCROLL_DY: i32 = 16; // 1px/frame placeholder
/// Score milestones that each grant one extra life (an "extend"). The exact
/// TH04 thresholds live in `MAIN.EXE`/the config; these are placeholders tuned
/// to this port's (still-simplified) scoring scale, applied cumulatively across
/// chained stages. TODO: replace with the ReC98 values once located.
const EXTEND_SCORES: &[i64] = &[10_000, 30_000, 70_000, 150_000, 300_000];
// Approximate enemy hit half-extent.
const ENEMY_HIT: i32 = 16 * SUBPIXEL;
/// Player kill-box half-extent — exact ReC98 value (`BULLET_KILLBOX_W/H =
/// TO_SP(8)`, `th04/main/bullet/bullet.hpp`): a bullet hits when the player's
/// centre is within 8px of the bullet centre on each axis.
const BULLET_KILL: i32 = 8 * SUBPIXEL;

/// A dropped item (power / point / …) falling for the player to collect.
#[derive(Debug, Clone, Copy)]
pub struct Item {
    pub x: i32,
    pub y: i32,
    pub vy: i32,
    pub kind: u8,
    pub active: bool,
}

/// High-level stage progression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Trash-wave timeline is running (a midboss interlude lives inside this).
    Trash,
    /// Timeline exhausted + field clear → the boss would appear here (TODO).
    Boss,
    /// Stage finished (boss defeated / nothing left).
    Cleared,
}

pub struct StageSim {
    std: Std,
    events: Vec<TimelineFrame>,
    ev_i: usize,
    pub frame: u16,
    pub enemies: Vec<Enemy>,
    pub bullets: BulletPool,
    pub player: Player,
    pub score: i64,
    pub enemies_spawned: u32,
    pub enemies_killed: u32,
    pub player_hits: u32,
    pub phase: Phase,
    pub boss: Option<Boss>,
    pub midboss: Option<Midboss>,
    pub items: Vec<Item>,
    /// Non-damaging boss telegraphs (gather/circle/spark markers).
    pub effects: EffectPool,
    /// Which boss spawns at the boss phase (`None` = no roster boss, e.g. Extra).
    boss_kind: Option<BossKind>,
    /// How many [`EXTEND_SCORES`] milestones have already granted a life.
    pub extends_awarded: usize,
    /// Frames remaining on the boss-appears name card (counts down from
    /// [`BOSS_INTRO_FRAMES`] when the boss spawns); 0 = not showing.
    pub boss_intro: u32,
    /// Player-death explosion: frames remaining (counts down from
    /// [`DEATH_FX_FRAMES`]) and where the player died. 0 = not showing.
    pub death_fx: u32,
    pub death_pos: (i32, i32),
    midboss_done: bool,
    rng: u32,
}

/// How long the boss name card shows when the boss appears.
pub const BOSS_INTRO_FRAMES: u32 = 140;
/// How long the player-death explosion plays.
pub const DEATH_FX_FRAMES: u32 = 28;

impl StageSim {
    /// Build a sim for a stage. `boss_kind` is the end-of-stage boss
    /// ([`BossKind::for_stage`]); pass `None` for a stage with no roster boss.
    pub fn new(std: Std, shot_type: u8, boss_kind: Option<BossKind>) -> Self {
        let events = std.timeline_events();
        StageSim {
            std,
            events,
            ev_i: 0,
            frame: 0,
            enemies: Vec::new(),
            bullets: BulletPool::new(),
            player: Player::new(shot_type),
            score: 0,
            enemies_spawned: 0,
            enemies_killed: 0,
            player_hits: 0,
            phase: Phase::Trash,
            boss: None,
            midboss: None,
            items: Vec::new(),
            effects: EffectPool::new(),
            boss_kind,
            extends_awarded: 0,
            boss_intro: 0,
            death_fx: 0,
            death_pos: (0, 0),
            midboss_done: false,
            rng: 0x9e37_79b9,
        }
    }

    /// Set the difficulty rank (0 = Easy … 3 = Lunatic); scales enemy/boss
    /// bullet counts (`bullet_template_tune`). Defaults to Normal.
    pub fn set_rank(&mut self, rank: u8) {
        self.bullets.set_rank(rank);
    }

    /// Carry a continuing run's state into this (freshly built) stage: the
    /// player's lives/bombs/power, the accumulated score, and how many extends
    /// have been awarded (so milestones aren't re-granted next stage).
    pub fn restore(&mut self, lives: i32, bombs: i32, power: u8, score: i64, extends_awarded: usize) {
        self.player.lives = lives;
        self.player.bombs = bombs;
        self.player.power = power;
        self.score = score;
        self.extends_awarded = extends_awarded;
    }

    /// Award an extra life for each [`EXTEND_SCORES`] milestone the score has
    /// reached since the last check.
    fn award_extends(&mut self) {
        while self.extends_awarded < EXTEND_SCORES.len()
            && self.score >= EXTEND_SCORES[self.extends_awarded]
        {
            self.player.lives += 1;
            self.extends_awarded += 1;
        }
    }

    /// Resolve a spawn coordinate, replacing ENEMY_POS_RANDOM with a random
    /// position within `bound` subpixels.
    fn resolve_pos(&mut self, v: i32, bound: i32) -> i32 {
        if v == ENEMY_POS_RANDOM {
            self.rng = self.rng.wrapping_mul(1103515245).wrapping_add(12345);
            ((self.rng >> 16) as i32).rem_euclid(bound)
        } else {
            v
        }
    }

    pub fn alive_enemies(&self) -> usize {
        self.enemies.iter().filter(|e| !e.killed).count()
    }

    /// True once the stage is cleared or the player is out of lives.
    pub fn finished(&self) -> bool {
        self.phase == Phase::Cleared || self.player.gameover
    }

    /// Advance the whole stage one frame.
    pub fn step(&mut self, input: &Input) {
        self.player.update(input);
        // The deathbomb window expired without a bomb → the player just died:
        // count the miss, burst the death explosion (at the recorded hit spot),
        // and clear the screen of bullets (TH04 behaviour).
        if self.player.just_died {
            self.player_hits += 1;
            self.death_fx = DEATH_FX_FRAMES;
            for b in self.bullets.bullets.iter_mut() {
                b.active = false;
            }
        }

        // Bomb: while active, keep the screen clear of enemy bullets and chip
        // away at everything on the field.
        if self.player.bombing() {
            for b in self.bullets.bullets.iter_mut() {
                b.active = false;
            }
            for e in self.enemies.iter_mut() {
                if e.can_be_damaged {
                    e.hp -= 4;
                    if e.hp <= 0 && !e.killed {
                        e.killed = true;
                        self.enemies_killed += 1;
                        self.score += e.score as i64;
                        self.effects.spark(e.x, e.y); // death puff
                    }
                }
            }
            if let Some(b) = self.boss.as_mut() {
                b.damage(2);
            }
            if let Some(m) = self.midboss.as_mut() {
                m.damage(2);
            }
        }

        // 1. Trash timeline (the midboss interrupts it mid-stage).
        if self.phase == Phase::Trash {
            if !self.midboss_done && self.midboss.is_none() && self.frame >= MIDBOSS_FRAME {
                self.midboss = Some(Midboss::new(192 * SUBPIXEL));
            }
            // Timeline pauses while the midboss is on screen.
            if self.midboss.is_none() {
                while self.ev_i < self.events.len() && self.events[self.ev_i].frame <= self.frame {
                    // Collect this frame's spawns first (resolve_pos needs &mut self).
                    let spawns = self.events[self.ev_i].spawns.clone();
                    for sp in &spawns {
                        let ex = self.resolve_pos(sp.x as i32, PLAYFIELD_W * SUBPIXEL);
                        let ey = self.resolve_pos(sp.y as i32, PLAYFIELD_H * SUBPIXEL);
                        let mut e = Enemy::spawn(ex, ey);
                        e.script_index = sp.script_index as usize;
                        e.item = sp.arg;
                        self.enemies.push(e);
                        self.enemies_spawned += 1;
                    }
                    self.ev_i += 1;
                }
            }
        }

        // 2. Run each enemy's script (movement + firing). Disjoint field
        //    borrows: enemies (mut), std (shared), bullets (mut).
        let player_pos = (self.player.x, self.player.y);
        for e in self.enemies.iter_mut() {
            if e.killed {
                continue;
            }
            let script = self
                .std
                .enemy_scripts
                .get(e.script_index)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            e.step(script, SCROLL_DY, player_pos, &mut self.bullets);
        }

        // 2b. Boss (during the boss phase).
        if self.phase == Phase::Boss {
            let frame = self.frame;
            if let Some(b) = self.boss.as_mut() {
                let mut env = BossEnv {
                    pool: &mut self.bullets,
                    fx: &mut self.effects,
                    player: player_pos,
                    frame,
                };
                b.update(&mut env);
            }
        }
        // 2c. Midboss.
        if let Some(m) = self.midboss.as_mut() {
            m.update(&mut self.bullets);
        }

        // 3. Move bullets + age boss telegraphs.
        self.bullets.update(player_pos, self.frame);
        self.effects.update();

        // 4. Player shots vs enemies.
        for s in self.player.shots.iter_mut() {
            if !s.active {
                continue;
            }
            for e in self.enemies.iter_mut() {
                if e.killed || !e.can_be_damaged {
                    continue;
                }
                if (s.x - e.x).abs() < ENEMY_HIT && (s.y - e.y).abs() < ENEMY_HIT {
                    e.hp -= s.damage as i16;
                    if e.hp <= 0 {
                        e.killed = true;
                        self.enemies_killed += 1;
                        self.score += e.score as i64;
                        self.effects.spark(e.x, e.y); // death puff
                        // Drop an item (kind from the spawn; default to a point
                        // item). The exact per-enemy drop table is a refinement.
                        let kind = if e.item == 0xFF { 1 } else { e.item };
                        self.items.push(Item { x: e.x, y: e.y, vy: -8, kind, active: true });
                    }
                    // A piercing laser keeps going (and can hit more enemies);
                    // an ordinary shot is consumed on the first hit.
                    if !s.pierce {
                        s.active = false;
                        break;
                    }
                }
            }
        }

        // 4b. Player shots vs boss / midboss.
        if let Some(b) = self.boss.as_mut() {
            if !b.defeated {
                for s in self.player.shots.iter_mut() {
                    if s.active && (s.x - b.x).abs() < BOSS_HIT && (s.y - b.y).abs() < BOSS_HIT {
                        b.damage(s.damage);
                        if !s.pierce {
                            s.active = false;
                        }
                    }
                }
                // Destructible satellites (Marisa's bits, hp > 0) can be shot
                // down; Reimu's orbs are invulnerable (hp 0) and skipped.
                let r = 12 * SUBPIXEL;
                for o in b.orbits.iter_mut().filter(|o| (1..=3).contains(&o.flag) && o.hp > 0) {
                    for s in self.player.shots.iter_mut() {
                        if s.active && (s.x - o.cx).abs() < r && (s.y - o.cy).abs() < r {
                            o.hp -= s.damage;
                            s.active = false;
                            if o.hp <= 0 {
                                o.flag = 0;
                            }
                        }
                    }
                }
            }
        }
        if let Some(m) = self.midboss.as_mut() {
            if !m.defeated {
                for s in self.player.shots.iter_mut() {
                    if s.active && (s.x - m.x).abs() < MIDBOSS_HIT && (s.y - m.y).abs() < MIDBOSS_HIT {
                        m.damage(s.damage);
                        if !s.pierce {
                            s.active = false;
                        }
                    }
                }
            }
        }

        // 5. Enemy bullets / bodies vs player (only when vulnerable).
        if !self.player.invincible() && !self.player.gameover {
            let (px, py) = (self.player.x, self.player.y);
            let mut died = false;
            for b in self.bullets.bullets.iter_mut() {
                if b.active && (b.x - px).abs() < BULLET_KILL && (b.y - py).abs() < BULLET_KILL {
                    b.active = false;
                    died = true;
                    break;
                }
            }
            if !died {
                for e in &self.enemies {
                    if !e.killed && e.kills_player && (e.x - px).abs() < ENEMY_HIT && (e.y - py).abs() < ENEMY_HIT {
                        died = true;
                        break;
                    }
                }
            }
            if !died {
                if let Some(b) = &self.boss {
                    if !b.defeated && (b.x - px).abs() < BOSS_HIT && (b.y - py).abs() < BOSS_HIT {
                        died = true;
                    }
                    // Orbs/bits are solid: touching one kills the player.
                    let r = 12 * SUBPIXEL;
                    if !died
                        && b.orbits.iter().any(|o| {
                            (1..=3).contains(&o.flag) && (o.cx - px).abs() < r && (o.cy - py).abs() < r
                        })
                    {
                        died = true;
                    }
                }
            }
            if !died {
                if let Some(m) = &self.midboss {
                    if !m.defeated && (m.x - px).abs() < MIDBOSS_HIT && (m.y - py).abs() < MIDBOSS_HIT {
                        died = true;
                    }
                }
            }
            if died {
                // Open the deathbomb window; the death only commits if the
                // player doesn't bomb in time (handled in `Player::update`,
                // observed via `just_died` at the top of the next frame).
                self.death_pos = (px, py);
                self.player.begin_dying();
            }
        }

        // 5b. Items fall (initial upward pop, then gravity) and auto-collect.
        {
            let (px, py) = (self.player.x, self.player.y);
            for it in self.items.iter_mut() {
                it.vy = (it.vy + 1).min(40);
                it.y += it.vy;
                if (it.x - px).abs() < 24 * SUBPIXEL && (it.y - py).abs() < 24 * SUBPIXEL {
                    it.active = false;
                    // Apply the item by its ReC98 kind (item_type_t).
                    use crate::player::{item, BIGPOWER_PER_ITEM, POWER_MAX, POWER_PER_ITEM};
                    match it.kind {
                        item::POWER => self.player.add_power(POWER_PER_ITEM),
                        item::BIGPOWER => self.player.add_power(BIGPOWER_PER_ITEM),
                        item::FULLPOWER => self.player.add_power(POWER_MAX),
                        item::BOMB => self.player.bombs += 1,
                        item::ONEUP => self.player.lives += 1,
                        // POINT / DREAM (and anything else) → score.
                        _ => self.score += 100,
                    }
                } else if it.y > 420 * SUBPIXEL {
                    it.active = false;
                }
            }
            self.items.retain(|i| i.active);
        }

        // 6. Drop dead enemies, advance time.
        self.enemies.retain(|e| !e.killed);

        // Midboss defeated → resume the trash timeline.
        if self.midboss.as_ref().map(Midboss::done).unwrap_or(false) {
            self.midboss = None;
            self.midboss_done = true;
        }

        // 7. Stage progression. After the midboss, once the trash timeline is
        //    exhausted and the field is clear, the boss appears; once defeated,
        //    the stage clears.
        if self.phase == Phase::Trash
            && self.midboss_done
            && self.ev_i >= self.events.len()
            && self.enemies.is_empty()
        {
            self.phase = Phase::Boss;
            self.boss = self.boss_kind.map(Boss::from_kind);
            if self.boss.is_some() {
                self.boss_intro = BOSS_INTRO_FRAMES;
            }
        }
        self.boss_intro = self.boss_intro.saturating_sub(1);
        self.death_fx = self.death_fx.saturating_sub(1);
        if self.phase == Phase::Boss && self.boss.as_ref().map(Boss::done).unwrap_or(true) {
            self.phase = Phase::Cleared;
        }

        // Extra lives for score milestones reached this frame.
        self.award_extends();

        self.frame = self.frame.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage::Std;

    fn tiny_stage() -> Std {
        // 1 enemy script: stats(hp=1) then end; timeline: frame 0 spawns it.
        let enemy_script = vec![0x10u8, 0, 1, 0, 5, 0, 0x00]; // stats patnum0 hp1 score5; end
        let mut std = Std {
            map_section_order: vec![],
            scroll_speeds: vec![],
            enemy_scripts: vec![enemy_script],
            timeline: vec![],
        };
        // timeline: frame 1, count 1, spawn script#0 at (100,50) item 0
        let mut tl = Vec::new();
        tl.extend_from_slice(&1u16.to_le_bytes());
        tl.push(1);
        tl.extend_from_slice(&[0, 100, 0, 50, 0, 0, 0, 0]); // script0, x=100, y=50
        tl.extend_from_slice(&0u16.to_le_bytes());
        std.timeline = tl;
        std
    }

    #[test]
    fn spawns_and_kills_enemy() {
        let mut sim = StageSim::new(tiny_stage(), 0, Some(BossKind::Orange));
        // place a player shot right on the enemy spawn and run a few frames
        let mut input = Input::default();
        input.shoot = true;
        for _ in 0..3 {
            sim.step(&input);
        }
        assert!(sim.enemies_spawned >= 1, "enemy should have spawned");
    }

    #[test]
    fn item_kinds_apply_their_effects() {
        use crate::player::{item, POWER_MAX};
        let mut sim = StageSim::new(tiny_stage(), 0, None);
        let (px, py) = (sim.player.x, sim.player.y);
        let (bombs0, lives0) = (sim.player.bombs, sim.player.lives);
        for kind in [item::BOMB, item::ONEUP, item::FULLPOWER] {
            sim.items.push(Item { x: px, y: py, vy: 0, kind, active: true });
        }
        sim.step(&Input::default());
        assert_eq!(sim.player.bombs, bombs0 + 1, "bomb item");
        assert_eq!(sim.player.lives, lives0 + 1, "1up item");
        assert_eq!(sim.player.power, POWER_MAX, "fullpower item");
    }

    #[test]
    fn boss_phase_spawns_the_selected_boss() {
        // Empty timeline → once the midboss interlude is done and the field is
        // clear, the boss phase spawns whatever kind the stage selected.
        let std = Std {
            map_section_order: vec![],
            scroll_speeds: vec![],
            enemy_scripts: vec![],
            timeline: vec![0, 0, 0], // frame 0 → end-of-timeline immediately
        };
        let mut sim = StageSim::new(std, 0, Some(BossKind::Elly));
        // Skip the midboss interlude (its activation frame is a placeholder).
        sim.midboss_done = true;
        sim.player.lives = 9; // survive long enough to reach the boss
        for _ in 0..16000 {
            let mut input = Input::default();
            input.shoot = true;
            sim.step(&input);
            if sim.phase == Phase::Boss {
                break;
            }
        }
        assert_eq!(sim.phase, Phase::Boss, "should reach the boss phase");
        assert!(sim.boss.is_some(), "the selected boss should spawn");
    }

    #[test]
    fn score_milestone_grants_an_extend() {
        let mut sim = StageSim::new(tiny_stage(), 0, None);
        let lives0 = sim.player.lives;
        // Cross the first extend threshold directly.
        sim.score = EXTEND_SCORES[0];
        sim.award_extends();
        assert_eq!(sim.player.lives, lives0 + 1);
        assert_eq!(sim.extends_awarded, 1);
        // No double-award while still below the next milestone.
        sim.award_extends();
        assert_eq!(sim.player.lives, lives0 + 1);
    }

    #[test]
    fn restore_carries_progress_without_regranting_extends() {
        let mut sim = StageSim::new(tiny_stage(), 0, None);
        sim.restore(5, 2, 64, EXTEND_SCORES[0] + 1, 1);
        sim.award_extends();
        // Already-awarded milestone must not grant another life.
        assert_eq!(sim.player.lives, 5);
        assert_eq!(sim.player.power, 64);
        assert_eq!(sim.score, EXTEND_SCORES[0] + 1);
    }

    #[test]
    fn extra_stage_has_no_roster_boss() {
        let std = Std {
            map_section_order: vec![],
            scroll_speeds: vec![],
            enemy_scripts: vec![],
            timeline: vec![0, 0, 0],
        };
        let mut sim = StageSim::new(std, 0, None);
        sim.midboss_done = true;
        for _ in 0..200 {
            sim.step(&Input::default());
            if sim.phase == Phase::Cleared {
                break;
            }
        }
        // No boss to fight → the stage clears straight through.
        assert!(sim.boss.is_none());
        assert_eq!(sim.phase, Phase::Cleared);
    }
}
