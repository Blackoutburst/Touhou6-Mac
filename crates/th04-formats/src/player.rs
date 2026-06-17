//! TH04 player: movement (ReC98 `player_move` + `player_pos_update_and_clamp`)
//! and a basic forward shot. Movement speed is a fixed 4px aligned / 3px
//! diagonal (TH04 notably has *no* focus slowdown). Position is clamped to the
//! playfield with the original margins.
//!
//! The per-character shot tables (Reimu A/B, Marisa A/B) and power tiers,
//! options, lasers and bombs are TODO — for now `shoot()` fires a simple
//! straight-up pair so the player is controllable in the simulation. Positions
//! are subpixels (16/px).

const SUBPIXEL: i32 = 16;
// TODO: confirm exact TH04 playfield.
pub const PLAYFIELD_W: i32 = 384;
pub const PLAYFIELD_H: i32 = 368;

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

/// Per-frame input (already debounced into held directions).
#[derive(Default, Clone, Copy, Debug)]
pub struct Input {
    pub left: bool,
    pub right: bool,
    pub up: bool,
    pub down: bool,
    pub shoot: bool,
    pub focus: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct PlayerShot {
    pub x: i32,
    pub y: i32,
    pub vx: i32,
    pub vy: i32,
    pub damage: i32,
    pub active: bool,
}

pub struct Player {
    pub x: i32,
    pub y: i32,
    pub lives: i32,
    pub bombs: i32,
    pub power: u8,
    pub shot_type: u8, // 0=ReimuA 1=ReimuB 2=MarisaA 3=MarisaB
    pub focused: bool,
    /// Frames of post-respawn invulnerability remaining (no hits while > 0).
    pub invuln: u32,
    pub gameover: bool,
    shot_timer: u8,
    pub shots: Vec<PlayerShot>,
}

/// Invulnerability granted on (re)spawn.
pub const RESPAWN_INVULN: u32 = 120;

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
            invuln: RESPAWN_INVULN,
            gameover: false,
            shot_timer: 0,
            shots: Vec::new(),
        }
    }

    fn start_pos() -> (i32, i32) {
        ((PLAYFIELD_W / 2) * SUBPIXEL, (PLAYFIELD_H - 48) * SUBPIXEL)
    }

    /// True while the player can't be hit.
    pub fn invincible(&self) -> bool {
        self.invuln > 0
    }

    /// Take a hit: lose a life and respawn, or set game over at < 0 lives.
    /// Returns true if the player died (caller may clear bullets, etc.).
    pub fn hit(&mut self) -> bool {
        if self.invincible() || self.gameover {
            return false;
        }
        self.lives -= 1;
        if self.lives < 0 {
            self.gameover = true;
        } else {
            let (x, y) = Self::start_pos();
            self.x = x;
            self.y = y;
            self.invuln = RESPAWN_INVULN;
        }
        true
    }

    pub fn active_shots(&self) -> usize {
        self.shots.iter().filter(|s| s.active).count()
    }

    fn add_shot(&mut self, x: i32, y: i32, vx: i32, vy: i32, damage: i32) {
        let s = PlayerShot { x, y, vx, vy, damage, active: true };
        match self.shots.iter_mut().find(|t| !t.active) {
            Some(slot) => *slot = s,
            None => self.shots.push(s),
        }
    }

    /// A basic forward shot. TODO: real per-character / per-power tables.
    fn fire(&mut self) {
        self.add_shot(self.x - 6 * SUBPIXEL, self.y, 0, -SHOT_SPEED, 1);
        self.add_shot(self.x + 6 * SUBPIXEL, self.y, 0, -SHOT_SPEED, 1);
    }

    /// Advance one frame: move + clamp, fire on cadence, update shots.
    pub fn update(&mut self, input: &Input) {
        if self.invuln > 0 {
            self.invuln -= 1;
        }
        if self.gameover {
            return;
        }
        self.focused = input.focus;

        let dx = input.right as i32 - input.left as i32;
        let dy = input.down as i32 - input.up as i32;
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
