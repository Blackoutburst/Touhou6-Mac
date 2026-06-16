//! Stateful execution of the enemy-script bytecode disassembled by [`crate::enemy`].
//!
//! This is the runtime half of the enemy VM (ReC98 interpreter `sub_155DD`):
//! it holds the live enemy state (position, velocity, angle/speed, script
//! pointer, blocking-frame counter, loop counter, bullet template) and advances
//! it one game frame at a time, faithfully reproducing the original control
//! flow — *blocking* instructions repeat their per-frame action for an
//! operand-given number of frames before advancing, *immediate* instructions
//! execute and fall through to the next instruction in the same frame.
//!
//! Motion uses TH04's 256-direction trig (master.lib `CosTable8`/`SinTable8`,
//! 8.8 fixed) exactly as `vector2_near`: `vx = cos8(angle)·speed >> 8`.
//!
//! Bullet opcodes update the enemy's [`BulletTemplate`] and record fire events;
//! actually spawning bullet objects is the bullet-system's job (next step), so
//! `fire()` here just counts. Positions are subpixels (16 per pixel).

use crate::enemy::op_len;

const SUBPIXEL: i32 = 16;
const ENEMY_W: i32 = 32;
const ENEMY_H: i32 = 32;
// TH04 playfield. TODO: confirm exact values from ReC98; only affects the frame
// at which a clipped enemy is removed.
const PLAYFIELD_W: i32 = 384;
const PLAYFIELD_H: i32 = 368;

/// 8.8 cosine for a 256-direction angle (0 = +x/right, 64 = +y/down).
fn cos8(a: u8) -> i32 {
    (256.0 * (a as f64 * std::f64::consts::TAU / 256.0).cos()).round() as i32
}
fn sin8(a: u8) -> i32 {
    (256.0 * (a as f64 * std::f64::consts::TAU / 256.0).sin()).round() as i32
}
/// Angle (256-direction) of the vector (dx, dy), matching ReC98 `iatan2`.
fn iatan2(dy: i32, dx: i32) -> u8 {
    let t = ((dy as f64).atan2(dx as f64) / std::f64::consts::TAU * 256.0).round() as i32;
    (t & 0xff) as u8
}

/// Bullet spawn parameters an enemy script builds up before firing.
#[derive(Default, Debug, Clone, Copy)]
pub struct BulletTemplate {
    pub spawn_type: u8,
    pub origin_x: i16,
    pub origin_y: i16,
    pub group: u8,
    pub angle: u8,
    pub speed: u8,
    pub patnum: u8,
    pub count: u8,
    pub delta: u8,
}

pub struct Enemy {
    pub x: i32,
    pub y: i32, // subpixel position
    pub vx: i32,
    pub vy: i32,
    pub angle: u8,
    pub speed: i16,
    pub angle_delta: u8,
    pub script_ip: usize,
    pub cur_instr_frame: u8,
    pub loop_i: u8,
    pub hp: i16,
    pub score: i32,
    pub patnum_base: u8,
    pub anim_cels: u8,
    pub anim_frames_per_cel: u8,
    pub clip_x: bool,
    pub clip_y: bool,
    pub alive: bool,        // EF_ALIVE (damageable/collidable)
    pub killed: bool,       // script ended / clipped
    pub kills_player: bool,
    pub can_be_damaged: bool,
    pub autofire: bool,
    pub spawned_left_half: bool,
    pub bullet: BulletTemplate,
    /// Number of explicit `fire` opcodes executed (bullet objects TBD).
    pub fire_count: u32,
    /// Simple deterministic RNG stand-in for `rand_angle` (not ZUN's randring).
    rng: u32,
}

impl Enemy {
    /// Spawn an enemy at (x, y) subpixels running from the start of `script`.
    pub fn spawn(x: i32, y: i32) -> Self {
        Enemy {
            x,
            y,
            vx: 0,
            vy: 0,
            angle: 0,
            speed: 0,
            angle_delta: 0,
            script_ip: 0,
            cur_instr_frame: 0,
            loop_i: 0,
            hp: 1,
            score: 0,
            patnum_base: 0,
            anim_cels: 1,
            anim_frames_per_cel: 4,
            clip_x: false,
            clip_y: false,
            alive: false,
            killed: false,
            kills_player: false,
            can_be_damaged: false,
            autofire: false,
            spawned_left_half: x < (PLAYFIELD_W / 2) * SUBPIXEL,
            bullet: BulletTemplate::default(),
            fire_count: 0,
            rng: 0x1234,
        }
    }

    fn set_velocity(&mut self) {
        self.vx = (cos8(self.angle) * self.speed as i32) >> 8;
        self.vy = (sin8(self.angle) * self.speed as i32) >> 8;
    }

    fn rand8(&mut self) -> u8 {
        self.rng = self.rng.wrapping_mul(1103515245).wrapping_add(12345);
        (self.rng >> 16) as u8
    }

    /// Apply velocity; return true if a clipped enemy left the playfield.
    fn motion(&mut self) -> bool {
        self.x += self.vx;
        self.y += self.vy;
        let mut clipped = false;
        if self.clip_x {
            let xv = (self.x + (ENEMY_W / 2) * SUBPIXEL) as u32;
            if xv >= ((PLAYFIELD_W + ENEMY_W) * SUBPIXEL) as u32 {
                clipped = true;
            }
        }
        if self.clip_y {
            let yv = (self.y + (ENEMY_H / 2) * SUBPIXEL) as u32;
            if yv >= ((PLAYFIELD_H + ENEMY_H) * SUBPIXEL) as u32 {
                clipped = true;
            }
        }
        clipped
    }

    /// Run one game frame of `script`. `scroll_dy` is this frame's vertical
    /// scroll delta (subpixels), `player` the player position (subpixels).
    pub fn step(&mut self, script: &[u8], scroll_dy: i32, player: (i32, i32)) {
        if self.killed {
            return;
        }
        // Immediate instructions fall through within the same frame; a blocking
        // instruction ends the frame (returns).
        loop {
            let ip = self.script_ip;
            let op = match script.get(ip) {
                Some(&o) => o,
                None => {
                    self.killed = true;
                    return;
                }
            };
            let len = op_len(op).unwrap_or(1);
            let o = |i: usize| script.get(ip + 1 + i).copied().unwrap_or(0);
            let u16o = |i: usize| i16::from_le_bytes([o(i), o(i + 1)]);

            // --- blocking movers: do per-frame motion, advance after N frames ---
            macro_rules! block {
                ($frames:expr) => {{
                    if self.motion() {
                        self.killed = true;
                        return;
                    }
                    if self.cur_instr_frame >= ($frames) {
                        self.cur_instr_frame = 0;
                        self.script_ip += len;
                    } else {
                        self.cur_instr_frame += 1;
                    }
                    return;
                }};
            }

            match op {
                0x00 => {
                    self.killed = true;
                    return;
                }
                0x01 => {
                    if self.cur_instr_frame == 0 {
                        self.angle = o(0);
                        self.speed = o(1) as i16;
                        self.set_velocity();
                    }
                    block!(o(2));
                }
                0x02 => block!(o(0)),
                0x03 => {
                    if self.cur_instr_frame == 0 {
                        self.speed = o(0) as i16;
                        self.set_velocity();
                    }
                    block!(o(1));
                }
                0x04 | 0x05 => {
                    if self.cur_instr_frame == 0 {
                        self.angle = o(0);
                        self.speed = o(1) as i16;
                        self.angle_delta = o(2);
                    }
                    self.set_velocity();
                    if op == 0x05 {
                        self.vx += (o(3) as i8) as i32;
                        self.vy += (o(4) as i8) as i32;
                    }
                    let frames = if op == 0x05 { o(5) } else { o(3) };
                    let killed = self.motion();
                    self.angle = self.angle.wrapping_add(self.angle_delta);
                    if killed {
                        self.killed = true;
                        return;
                    }
                    if self.cur_instr_frame >= frames {
                        self.cur_instr_frame = 0;
                        self.script_ip += len;
                    } else {
                        self.cur_instr_frame += 1;
                    }
                    return;
                }
                0x06 => block!(o(0)),
                0x09 => {
                    // aim at player: angle = iatan2 + operand, speed = operand
                    self.angle = self.angle.wrapping_add(o(0));
                    self.speed = o(1) as i16;
                    let a = iatan2(player.1 - self.y, player.0 - self.x);
                    self.angle = a.wrapping_add(o(0));
                    self.set_velocity();
                    self.script_ip += len;
                    continue;
                }
                0x0A => {
                    self.angle = self.angle.wrapping_add(o(0));
                    self.set_velocity();
                    self.script_ip += len;
                    continue;
                }
                0x0B => {
                    if self.cur_instr_frame == 0 {
                        self.vx = 0;
                    }
                    self.vy = scroll_dy;
                    block!(o(0));
                }
                0x0C => {
                    self.speed = self.speed.wrapping_add((o(0) as i8) as i16);
                    self.set_velocity();
                    self.script_ip += len;
                    continue;
                }
                0x0D | 0x0E => {
                    self.set_velocity();
                    let frames;
                    if op == 0x0E {
                        self.vx += (o(0) as i8) as i32;
                        self.vy += (o(1) as i8) as i32;
                        frames = o(2);
                    } else {
                        frames = o(0);
                    }
                    let killed = self.motion();
                    self.angle = self.angle.wrapping_add(self.angle_delta);
                    if killed {
                        self.killed = true;
                        return;
                    }
                    if self.cur_instr_frame >= frames {
                        self.cur_instr_frame = 0;
                        self.script_ip += len;
                    } else {
                        self.cur_instr_frame += 1;
                    }
                    return;
                }
                0x10 => {
                    self.patnum_base = o(0);
                    self.hp = u16o(1);
                    self.score = u16o(3) as i32;
                    self.alive = true;
                    self.can_be_damaged = true;
                    self.kills_player = true;
                    self.script_ip += len;
                    continue;
                }
                0x11 => {
                    self.angle = self.rand8();
                    self.script_ip += len;
                    continue;
                }
                0x12 => {
                    self.angle = o(0);
                    self.speed = o(1) as i16;
                    self.set_velocity();
                    self.script_ip += len;
                    continue;
                }
                0x13 => {
                    self.angle = o(0);
                    self.speed = o(1) as i16;
                    if !self.spawned_left_half {
                        self.angle = 0x80u8.wrapping_sub(self.angle);
                    }
                    self.set_velocity();
                    self.script_ip += len;
                    continue;
                }
                0x14 => {
                    self.speed = o(0) as i16;
                    self.set_velocity();
                    self.script_ip += len;
                    continue;
                }
                // --- bullets: build template / fire (objects TBD) ---
                0x20 => {
                    self.fire_count += 1;
                    self.script_ip += len;
                    continue;
                }
                0x21 => {
                    self.autofire = false;
                    self.bullet.spawn_type = o(0);
                    self.bullet.origin_x = u16o(1);
                    self.bullet.origin_y = u16o(3);
                    self.bullet.group = o(5);
                    self.bullet.angle = o(6);
                    self.bullet.speed = o(7);
                    self.bullet.patnum = o(8);
                    self.bullet.count = o(9);
                    self.script_ip += len;
                    continue;
                }
                0x22 => { self.bullet.spawn_type = o(0); self.script_ip += len; continue; }
                0x23 => { self.bullet.origin_x = u16o(0); self.bullet.origin_y = u16o(2); self.script_ip += len; continue; }
                0x24 => { self.bullet.angle = o(0); self.script_ip += len; continue; }
                0x25 => { self.bullet.angle = self.bullet.angle.wrapping_add(o(0)); self.script_ip += len; continue; }
                0x26 => { self.bullet.speed = o(0); self.script_ip += len; continue; }
                0x27 => { self.bullet.speed = self.bullet.speed.wrapping_add(o(0)); self.script_ip += len; continue; }
                0x28 => { self.bullet.group = o(0); self.script_ip += len; continue; }
                0x29 => { self.bullet.count = o(0); self.script_ip += len; continue; }
                0x2A => { self.bullet.patnum = o(0); self.script_ip += len; continue; }
                0x2B => { self.autofire = true; self.script_ip += len; continue; }
                0x2C => { /* autofire interval (rank/perf adjusted) */ self.script_ip += len; continue; }
                0x2D => { self.bullet.angle = self.rand8(); self.script_ip += len; continue; }
                0x2E => { self.autofire = false; self.script_ip += len; continue; }
                0x30 => { self.bullet.delta = o(0); self.script_ip += len; continue; }
                // --- control ---
                0x80 | 0x81 => {
                    if self.loop_i < o(1) {
                        self.loop_i += 1;
                        if op == 0x80 {
                            self.script_ip = o(0) as usize; // absolute
                        } else {
                            self.script_ip = self.script_ip.saturating_sub(o(0) as usize); // relative back
                        }
                    } else {
                        self.loop_i = 0;
                        self.script_ip += len;
                    }
                    continue;
                }
                0x82 => { self.clip_x = true; self.script_ip += len; continue; }
                0x83 => { self.clip_y = true; self.script_ip += len; continue; }
                0x84 => { self.clip_x = true; self.clip_y = true; self.script_ip += len; continue; }
                0x85 => { self.anim_cels = o(0); self.anim_frames_per_cel = o(1); self.script_ip += len; continue; }
                0x86 => { /* play SE */ self.script_ip += len; continue; }
                0x87 => { self.patnum_base = o(0); self.script_ip += len; continue; }
                0x88 => { self.can_be_damaged = false; self.autofire = false; self.script_ip += len; continue; }
                0x89 => { self.can_be_damaged = true; self.script_ip += len; continue; }
                0x8A => {
                    self.x = u16o(0) as i32;
                    self.y = u16o(2) as i32;
                    self.script_ip += len;
                    return; // 1-frame (var_1 = 0)
                }
                0x8B => {
                    self.x += u16o(0) as i32;
                    self.y += u16o(2) as i32;
                    self.script_ip += len;
                    return;
                }
                0x8C => { self.kills_player = false; self.script_ip += len; continue; }
                0x8D => { self.kills_player = true; self.script_ip += len; continue; }
                0x8E => { self.patnum_base = self.patnum_base.wrapping_add(o(0)); self.script_ip += len; continue; }
                0x8F => { self.script_ip += len; continue; } // tile-ring set (visual)
                _ => {
                    // Undefined opcode: stop to avoid desync.
                    self.killed = true;
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_straight_down() {
        // move(angle=64 [down], speed=32, frames=10) ; end
        let script = [0x01u8, 64, 32, 10, 0x00];
        let mut e = Enemy::spawn(0, 0);
        for _ in 0..5 {
            e.step(&script, 0, (0, 0));
        }
        assert!(e.y > 0, "should move down (+y)");
        assert_eq!(e.vx, 0, "no horizontal drift at angle 64");
        assert!(!e.killed);
    }

    #[test]
    fn fire_then_end() {
        // fire ; end
        let script = [0x20u8, 0x00];
        let mut e = Enemy::spawn(0, 0);
        e.step(&script, 0, (0, 0));
        assert_eq!(e.fire_count, 1);
        assert!(e.killed);
    }
}
