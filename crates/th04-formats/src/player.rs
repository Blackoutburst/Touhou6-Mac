//! TH04 player: movement (ReC98 `player_move` + `player_pos_update_and_clamp`)
//! and a basic forward shot. Movement speed is a fixed 4px aligned / 3px
//! diagonal (TH04 notably has *no* focus slowdown). Position is clamped to the
//! playfield with the original margins.
//!
//! The per-character shot tables (Reimu A/B, Marisa A/B) and power tiers,
//! options, lasers and bombs are TODO — for now `shoot()` fires a simple
//! straight-up pair so the player is controllable in the simulation. Positions
//! are subpixels (16/px).

use crate::math::vector2;

const SUBPIXEL: i32 = 16;
// TODO: confirm exact TH04 playfield.
pub const PLAYFIELD_W: i32 = 384;
pub const PLAYFIELD_H: i32 = 368;
/// "Up" in the 256-direction system (0 = +x, 64 = +y/down, 192 = up).
const ANGLE_UP: u8 = 192;
const SHOT_DAMAGE: i32 = 10; // RE: th04 player shots deal 10
/// Bomb: invulnerable + clears bullets + damages everything for this long.
pub const BOMB_FRAMES: u32 = 180;

// ReC98 th04/main/player/move.hpp: TO_SP(4) / TO_SP(3).
const SPEED_ALIGNED: i32 = 4 * SUBPIXEL;
const SPEED_DIAGONAL: i32 = 3 * SUBPIXEL;
// ReC98 th04/main/player/pos.cpp clamp margins.
const MARGIN_L: i32 = 8;
const MARGIN_T: i32 = 8;
const MARGIN_R: i32 = 8;
const MARGIN_B: i32 = 16;
// Player shots travel 12px (ReC98 shot_velocity_set length); fire cadence.
const SHOT_SPEED: i32 = 12 * SUBPIXEL;
const SHOT_INTERVAL: u8 = 4;

pub const POWER_MAX: u8 = 128;
/// Power required for each shot level (ReC98 `_SHOT_LEVEL_TO_POWER`,
/// `th04/main/player/shot_levels[data].asm`): level 1 at power 6, 2 at 12, …,
/// 9 at 128. The shot level is how many of these thresholds the power has met.
const SHOT_LEVEL_TO_POWER: [u8; 9] = [6, 12, 16, 24, 32, 48, 72, 96, 128];
/// Power gained per small power item, and per big-power item. (ReC98 item kinds
/// are exact — see [`item`]; the exact power *amounts* are still approximate:
/// small = 1, big = 8.)
pub const POWER_PER_ITEM: u8 = 1;
pub const BIGPOWER_PER_ITEM: u8 = 8;

/// TH04 item kinds (`item_type_t`, `th04/main/item/item.hpp`).
pub mod item {
    pub const POWER: u8 = 0;
    pub const POINT: u8 = 1;
    pub const DREAM: u8 = 2;
    pub const BIGPOWER: u8 = 3;
    pub const BOMB: u8 = 4;
    pub const ONEUP: u8 = 5;
    pub const FULLPOWER: u8 = 6;
}
/// Power lost on a miss — TH04 drops the shot a tier when you die.
pub const POWER_LOSS_ON_DEATH: u8 = 16;

/// Per-frame input (already debounced into held directions).
#[derive(Default, Clone, Copy, Debug)]
pub struct Input {
    pub left: bool,
    pub right: bool,
    pub up: bool,
    pub down: bool,
    pub shoot: bool,
    pub focus: bool,
    pub bomb: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct PlayerShot {
    pub x: i32,
    pub y: i32,
    pub vx: i32,
    pub vy: i32,
    pub damage: i32,
    pub active: bool,
    /// Piercing (Marisa A's laser): stays alive through enemies instead of
    /// being consumed on the first hit.
    pub pierce: bool,
}

pub struct Player {
    pub x: i32,
    pub y: i32,
    pub lives: i32,
    pub bombs: i32,
    pub power: u8,
    pub shot_type: u8, // 0=ReimuA 1=ReimuB 2=MarisaA 3=MarisaB
    pub focused: bool,
    /// Horizontal lean for the banking sprite: -1 left, 0 neutral, +1 right.
    pub facing: i8,
    /// Frames of post-respawn invulnerability remaining (no hits while > 0).
    pub invuln: u32,
    pub gameover: bool,
    /// Frames of bomb remaining (clears bullets + damages while > 0).
    pub bombing: u32,
    /// Deathbomb window: frames left to bomb-cancel an incoming death (0 = not
    /// dying). The hit is committed only when this runs out.
    pub dying: u32,
    /// Set for the one frame the player actually loses a life (sim reads it).
    pub just_died: bool,
    shot_timer: u8,
    pub shots: Vec<PlayerShot>,
}

/// Invulnerability granted on (re)spawn.
pub const RESPAWN_INVULN: u32 = 120;
/// Deathbomb window: bombing within this many frames of being hit cancels the
/// death. (TH04 mechanic; exact length pending ReC98 — ~8 frames.)
pub const DEATHBOMB_FRAMES: u32 = 8;

impl Player {
    /// Start at the bottom centre of the playfield with the chosen shot type.
    pub fn new(shot_type: u8) -> Self {
        Player {
            x: (PLAYFIELD_W / 2) * SUBPIXEL,
            y: (PLAYFIELD_H - 48) * SUBPIXEL,
            lives: 2,
            bombs: 3,
            power: 0,
            shot_type,
            focused: false,
            facing: 0,
            invuln: RESPAWN_INVULN,
            gameover: false,
            bombing: 0,
            dying: 0,
            just_died: false,
            shot_timer: 0,
            shots: Vec::new(),
        }
    }

    /// True while a bomb is active.
    pub fn bombing(&self) -> bool {
        self.bombing > 0
    }

    fn start_pos() -> (i32, i32) {
        ((PLAYFIELD_W / 2) * SUBPIXEL, (PLAYFIELD_H - 48) * SUBPIXEL)
    }

    /// True while the player can't be hit (post-respawn, bombing, or already in
    /// the deathbomb window).
    pub fn invincible(&self) -> bool {
        self.invuln > 0 || self.bombing > 0 || self.dying > 0
    }

    /// A bullet/body reached the player: open the deathbomb window (the death is
    /// only committed when it expires, unless the player bombs first).
    pub fn begin_dying(&mut self) {
        if !self.invincible() && !self.gameover {
            self.dying = DEATHBOMB_FRAMES;
        }
    }

    /// Commit a death: lose a life + a power tier, respawn (or game over).
    fn do_death(&mut self) {
        self.lives -= 1;
        self.power = self.power.saturating_sub(POWER_LOSS_ON_DEATH);
        self.just_died = true;
        self.dying = 0;
        if self.lives < 0 {
            self.gameover = true;
        } else {
            let (x, y) = Self::start_pos();
            self.x = x;
            self.y = y;
            self.invuln = RESPAWN_INVULN;
        }
    }

    /// Kill the player immediately (skipping the deathbomb window). Returns true
    /// if the death was applied. Mostly for tests / forced deaths.
    pub fn hit(&mut self) -> bool {
        if self.invuln > 0 || self.bombing > 0 || self.gameover {
            return false;
        }
        self.do_death();
        true
    }

    pub fn active_shots(&self) -> usize {
        self.shots.iter().filter(|s| s.active).count()
    }

    /// Raise power by `amt` (collecting a power item), clamped to [`POWER_MAX`].
    pub fn add_power(&mut self, amt: u8) {
        self.power = self.power.saturating_add(amt).min(POWER_MAX);
    }

    /// Current shot level (0..=9) from power, via the exact ReC98
    /// [`SHOT_LEVEL_TO_POWER`] thresholds — drives [`Player::fire`].
    pub fn shot_level(&self) -> i32 {
        SHOT_LEVEL_TO_POWER.iter().filter(|&&t| self.power >= t).count() as i32
    }

    fn add_shot(&mut self, x: i32, y: i32, vx: i32, vy: i32, damage: i32) {
        self.add_shot_ex(x, y, vx, vy, damage, false);
    }

    fn add_shot_ex(&mut self, x: i32, y: i32, vx: i32, vy: i32, damage: i32, pierce: bool) {
        let s = PlayerShot { x, y, vx, vy, damage, active: true, pierce };
        match self.shots.iter_mut().find(|t| !t.active) {
            Some(slot) => *slot = s,
            None => self.shots.push(s),
        }
    }

    /// Fire the current frame's shots for the chosen character. Each of the four
    /// shot types has a distinct, per-level-scaling pattern (Reimu A wide fan,
    /// Reimu B focused needles, Marisa A piercing lasers + side amulets, Marisa
    /// B wide spread). Damage 10 / shot velocity 12px are exact (ReC98) and the
    /// shot *count* tracks the exact [`Player::shot_level`]; the precise angles
    /// are faithful-structure, not the byte-exact `th04_main.asm` `shot_*` tables.
    fn fire(&mut self) {
        let l = self.shot_level();
        match self.shot_type {
            0 => self.fire_reimu_a(l),
            1 => self.fire_reimu_b(l),
            2 => self.fire_marisa_a(l),
            _ => self.fire_marisa_b(l),
        }
    }

    /// Spawn one straight-up shot at an x-offset (subpixels) from the player.
    fn shot_up(&mut self, dx: i32, damage: i32, pierce: bool) {
        self.add_shot_ex(self.x + dx, self.y, 0, -SHOT_SPEED, damage, pierce);
    }
    /// Spawn one shot at an angle offset (256-dir units) from straight up.
    fn shot_fan(&mut self, off: i32, damage: i32) {
        let (vx, vy) = vector2(ANGLE_UP.wrapping_add(off as u8), SHOT_SPEED);
        self.add_shot(self.x, self.y, vx, vy, damage);
    }

    /// Reimu A — a forward amulet fan that widens with power.
    fn fire_reimu_a(&mut self, level: i32) {
        let n = 2 + level; // 2..11
        for i in 0..n {
            let off = 2 * i - (n - 1); // symmetric about 0
            self.shot_fan(off * 4, SHOT_DAMAGE); // 4 units apart — a wide spread
        }
    }

    /// Reimu B — focused needles: a tight, mostly-forward column that adds a
    /// narrow fan with power.
    fn fire_reimu_b(&mut self, level: i32) {
        let pairs = 1 + level; // straight needles down the centre
        for i in 0..pairs {
            let dx = (i - pairs / 2) * SUBPIXEL; // packed 1px apart
            self.shot_up(dx, SHOT_DAMAGE, false);
        }
        if level >= 4 {
            self.shot_fan(-2, SHOT_DAMAGE); // a slight fan at higher power
            self.shot_fan(2, SHOT_DAMAGE);
        }
    }

    /// Marisa A — central piercing laser(s) (more with power) flanked by amulets.
    fn fire_marisa_a(&mut self, level: i32) {
        let lasers = 1 + level / 3; // 1..4 piercing columns
        for i in 0..lasers {
            let dx = (2 * i - (lasers - 1)) * 4 * SUBPIXEL;
            self.shot_up(dx, SHOT_DAMAGE, true); // pierces enemies
        }
        // Side amulets fan out as power grows.
        for i in 0..level {
            let off = if i % 2 == 0 { -(i + 2) } else { i + 2 };
            self.shot_fan(off * 2, SHOT_DAMAGE);
        }
    }

    /// Marisa B — a wide spread of forward shots.
    fn fire_marisa_b(&mut self, level: i32) {
        let n = 2 + level;
        for i in 0..n {
            let off = 2 * i - (n - 1);
            // Wide columns spread across the front.
            self.shot_up(off * 4 * SUBPIXEL, SHOT_DAMAGE, false);
        }
    }

    /// Advance one frame: move + clamp, fire on cadence, update shots.
    pub fn update(&mut self, input: &Input) {
        self.just_died = false;
        if self.invuln > 0 {
            self.invuln -= 1;
        }
        if self.gameover {
            return;
        }
        // Deathbomb window: a bomb here cancels the incoming death; otherwise it
        // counts down and commits the death when it hits zero.
        if self.dying > 0 {
            if input.bomb && self.bombs > 0 {
                self.bombs -= 1;
                self.bombing = BOMB_FRAMES;
                self.dying = 0;
            } else {
                self.dying -= 1;
                if self.dying == 0 {
                    self.do_death();
                }
            }
        } else if self.bombing > 0 {
            // Bomb active: invulnerable; the sim clears bullets + damages.
            self.bombing -= 1;
        } else if input.bomb && self.bombs > 0 {
            self.bombs -= 1;
            self.bombing = BOMB_FRAMES;
        }
        if self.gameover {
            return;
        }
        self.focused = input.focus;

        let dx = input.right as i32 - input.left as i32;
        let dy = input.down as i32 - input.up as i32;
        self.facing = dx as i8;
        let (vx, vy) = if dx != 0 && dy != 0 {
            (dx * SPEED_DIAGONAL, dy * SPEED_DIAGONAL)
        } else {
            (dx * SPEED_ALIGNED, dy * SPEED_ALIGNED)
        };
        self.x = (self.x + vx).clamp(MARGIN_L * SUBPIXEL, (PLAYFIELD_W - MARGIN_R) * SUBPIXEL);
        self.y = (self.y + vy).clamp(MARGIN_T * SUBPIXEL, (PLAYFIELD_H - MARGIN_B) * SUBPIXEL);

        if input.shoot && self.shot_timer == 0 {
            self.fire();
            self.shot_timer = SHOT_INTERVAL;
        }
        if self.shot_timer > 0 {
            self.shot_timer -= 1;
        }

        for s in self.shots.iter_mut() {
            if s.active {
                s.x += s.vx;
                s.y += s.vy;
                if s.y < -16 * SUBPIXEL {
                    s.active = false;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_to_right_edge() {
        let mut p = Player::new(0);
        let mut input = Input::default();
        input.right = true;
        for _ in 0..300 {
            p.update(&input);
        }
        assert_eq!(p.x, (PLAYFIELD_W - MARGIN_R) * SUBPIXEL);
    }

    #[test]
    fn shooting_spawns_upward_shots() {
        let mut p = Player::new(0);
        let mut input = Input::default();
        input.shoot = true;
        p.update(&input);
        assert_eq!(p.active_shots(), 2);
        assert!(p.shots.iter().all(|s| s.vy < 0)); // travelling up
    }

    #[test]
    fn bomb_consumes_and_protects() {
        let mut p = Player::new(0);
        let bombs = p.bombs;
        let mut input = Input::default();
        input.bomb = true;
        p.update(&input);
        assert_eq!(p.bombs, bombs - 1);
        assert!(p.bombing());
        assert!(p.invincible());
    }

    #[test]
    fn marisa_a_fires_a_piercing_laser() {
        let mut p = Player::new(2); // Marisa A
        let mut input = Input::default();
        input.shoot = true;
        p.update(&input);
        // At least one shot is a straight-up piercing laser.
        assert!(p.active_shots() >= 1);
        assert!(p.shots.iter().filter(|s| s.active).any(|s| s.pierce && s.vx == 0 && s.vy < 0));
    }

    #[test]
    fn reimu_b_is_more_focused_than_marisa_b() {
        // At the same power, Reimu B's needles stay near the centre column while
        // Marisa B's spread fans wide.
        let level_power = SHOT_LEVEL_TO_POWER[4]; // a mid power level
        let spread = |shot_type: u8| {
            let mut p = Player::new(shot_type);
            p.add_power(level_power);
            let mut input = Input::default();
            input.shoot = true;
            p.update(&input);
            p.shots.iter().filter(|s| s.active).map(|s| (s.x - p.x).abs()).max().unwrap_or(0)
        };
        assert!(spread(1) < spread(3), "Reimu B should be tighter than Marisa B");
    }

    #[test]
    fn power_raises_shot_count_and_death_drops_it() {
        let mut p = Player::new(0);
        let mut input = Input::default();
        input.shoot = true;
        p.update(&input);
        let base = p.active_shots();
        // Power up to the top tier → more shots.
        p.add_power(POWER_MAX);
        assert_eq!(p.power, POWER_MAX);
        assert_eq!(p.shot_level(), 9); // full power = max level (ReC98 table)
        for s in p.shots.iter_mut() {
            s.active = false;
        }
        p.shot_timer = 0;
        p.update(&input);
        assert!(p.active_shots() > base, "more shots at full power");
        // A miss drops power a tier.
        p.invuln = 0;
        let pw = p.power;
        p.hit();
        assert_eq!(p.power, pw - POWER_LOSS_ON_DEATH);
    }

    #[test]
    fn deathbomb_within_window_saves_the_life() {
        let mut p = Player::new(0);
        p.invuln = 0; // make hittable
        let lives = p.lives;
        p.begin_dying();
        assert!(p.dying > 0);
        let mut input = Input::default();
        input.bomb = true;
        p.update(&input);
        assert_eq!(p.lives, lives, "deathbomb keeps the life");
        assert!(p.bombing());
        assert_eq!(p.dying, 0);
        assert!(!p.just_died);
        assert_eq!(p.bombs, 2);
    }

    #[test]
    fn missed_deathbomb_commits_the_death() {
        let mut p = Player::new(0);
        p.invuln = 0;
        let lives = p.lives;
        p.begin_dying();
        let input = Input::default(); // never bomb
        for _ in 0..DEATHBOMB_FRAMES {
            p.update(&input);
        }
        assert_eq!(p.lives, lives - 1, "death commits when the window expires");
        assert!(p.just_died);
    }

    #[test]
    fn diagonal_is_slower_per_axis() {
        let mut p = Player::new(0);
        let start = p.x;
        let mut input = Input::default();
        input.right = true;
        input.up = true; // diagonal
        p.update(&input);
        assert_eq!(p.x - start, SPEED_DIAGONAL);
    }
}
