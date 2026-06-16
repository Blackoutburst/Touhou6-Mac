//! TH04 enemy-script bytecode — the per-enemy VM that drives movement,
//! animation and bullet firing. Each enemy runs a script from
//! [`crate::stage::Std::enemy_scripts`]; the timeline spawns enemies pointing
//! at one.
//!
//! Reverse-engineered from the interpreter `sub_155DD` and its 0x90-entry jump
//! table (`off_15B4D`) in `th04_main.asm` (not in ReC98's decompiled C++). Each
//! instruction is `opcode` + fixed-length operands. The interpreter classes
//! opcodes as *blocking* (run for an operand-given number of frames before
//! advancing — movement, waits) or *immediate* (execute and fall through to the
//! next instruction the same frame — set-state, fire). `0x00` ends the script
//! (kills the enemy); `0x80`/`0x81` are loops.
//!
//! This module is the **disassembler**: it splits a script into typed
//! instructions with correct boundaries. The stateful VM that *executes* them
//! (applying motion, spawning bullets, ticking the blocking-frame counter) is
//! the next step and will match on these opcodes. Verified: every enemy script
//! in `ST00..ST06.STD` disassembles cleanly and terminates at `END`.

/// Total length of an instruction (opcode + operands), or `None` for opcodes
/// that don't appear in TH04 scripts (the VM maps them to a no-op tail).
pub fn op_len(op: u8) -> Option<usize> {
    Some(match op {
        // movement / motion
        0x00 => 1, // END (kills the enemy)
        0x01 => 4, // move: angle, speed, frames
        0x02 => 2, // move with current velocity: frames
        0x03 => 3, // set speed + move: speed, frames
        0x04 => 5, // set angle/speed/angle_delta: angle, speed, delta, frames
        0x05 => 7, // 0x04 + velocity bias: + dvx, dvy, (frames at +6)
        0x06 => 2, // hold position: frames
        0x07 => 5, // velocity from magnitude·cos(angle): mag, delta, vy, frames
        0x08 => 5, // 0x07 with x/y swapped
        0x09 => 3, // aim at player: angle offset, speed
        0x0A => 2, // turn: angle += d
        0x0B => 2, // follow vertical scroll: frames
        0x0C => 2, // accelerate: speed += d (signed)
        0x0D => 2, // move (delta) : frames; angle += angle_delta
        0x0E => 4, // 0x0D + velocity bias: dvx, dvy, frames
        0x10 => 6, // stats: patnum_base, hp(u16), score(u16); becomes alive/damageable
        0x11 => 1, // angle = random
        0x12 => 3, // set angle, speed (immediate)
        0x13 => 3, // set angle (mirrored if spawned right-half), speed
        0x14 => 2, // set speed (immediate)
        // bullets
        0x20 => 1,  // FIRE bullets from the template
        0x21 => 11, // set bullet template: type, ox(u16), oy(u16), group, angle, speed, patnum, count
        0x22 => 2,  // bullet spawn type
        0x23 => 5,  // bullet origin x(u16), y(u16)
        0x24 => 2,  // bullet angle =
        0x25 => 2,  // bullet angle +=
        0x26 => 2,  // bullet speed =
        0x27 => 2,  // bullet speed +=
        0x28 => 2,  // bullet group =
        0x29 => 2,  // bullet count =
        0x2A => 2,  // bullet patnum =
        0x2B => 1,  // autofire on
        0x2C => 2,  // autofire interval = (rank/perf-adjusted)
        0x2D => 1,  // bullet angle = random
        0x2E => 1,  // autofire off
        0x30 => 2,  // bullet angle delta (BT_delta)
        // control / state
        0x80 => 3, // loop: target(abs), count
        0x81 => 3, // loop: offset(back), count
        0x82 => 1, // clip x
        0x83 => 1, // clip y
        0x84 => 1, // clip x+y
        0x85 => 3, // animation: cels, frames_per_cel
        0x86 => 2, // play sound effect
        0x87 => 2, // patnum_base =
        0x88 => 1, // invulnerable (can't be damaged, autofire off)
        0x89 => 1, // vulnerable
        0x8A => 5, // set position x(u16), y(u16)
        0x8B => 5, // move position by x(u16), y(u16)
        0x8C => 1, // no longer kills player on collision
        0x8D => 1, // kills player on collision
        0x8E => 2, // patnum_base +=
        0x8F => 2, // set tile-ring image
        _ => return None,
    })
}

/// Human-readable mnemonic for an opcode.
pub fn op_name(op: u8) -> &'static str {
    match op {
        0x00 => "end",
        0x01 => "move",
        0x02 => "move_cur",
        0x03 => "set_speed_move",
        0x04 => "set_avd",
        0x05 => "set_avd_bias",
        0x06 => "hold",
        0x07 => "vel_mag",
        0x08 => "vel_mag_swap",
        0x09 => "aim_player",
        0x0A => "turn",
        0x0B => "follow_scroll",
        0x0C => "accel",
        0x0D => "move_delta",
        0x0E => "move_delta_bias",
        0x10 => "stats",
        0x11 => "rand_angle",
        0x12 => "set_av",
        0x13 => "set_av_mirror",
        0x14 => "set_speed",
        0x20 => "fire",
        0x21 => "bt_set",
        0x22 => "bt_type",
        0x23 => "bt_origin",
        0x24 => "bt_angle",
        0x25 => "bt_angle_add",
        0x26 => "bt_speed",
        0x27 => "bt_speed_add",
        0x28 => "bt_group",
        0x29 => "bt_count",
        0x2A => "bt_patnum",
        0x2B => "autofire_on",
        0x2C => "autofire_interval",
        0x2D => "bt_angle_rand",
        0x2E => "autofire_off",
        0x30 => "bt_delta",
        0x80 => "loop_abs",
        0x81 => "loop_rel",
        0x82 => "clip_x",
        0x83 => "clip_y",
        0x84 => "clip_xy",
        0x85 => "anim",
        0x86 => "se",
        0x87 => "patnum",
        0x88 => "invuln",
        0x89 => "vuln",
        0x8A => "set_pos",
        0x8B => "move_pos",
        0x8C => "no_kill",
        0x8D => "kill_on_touch",
        0x8E => "patnum_add",
        0x8F => "tile_set",
        _ => "?",
    }
}

/// One decoded instruction.
#[derive(Debug, Clone)]
pub struct Insn {
    pub offset: usize,
    pub opcode: u8,
    pub mnemonic: &'static str,
    /// Operand bytes (everything after the opcode byte).
    pub operands: Vec<u8>,
}

/// Disassemble an enemy script into instructions. Stops at the `END` opcode
/// (0x00) or when it hits an opcode with no defined length (which never happens
/// in valid TH04 scripts). `complete` is true when the walk reached an `END`.
pub fn disassemble(script: &[u8]) -> (Vec<Insn>, bool) {
    let mut out = Vec::new();
    let mut ip = 0usize;
    let mut complete = false;
    while ip < script.len() {
        let opcode = script[ip];
        let len = match op_len(opcode) {
            Some(l) => l,
            None => break,
        };
        let end = (ip + len).min(script.len());
        out.push(Insn {
            offset: ip,
            opcode,
            mnemonic: op_name(opcode),
            operands: script[ip + 1..end].to_vec(),
        });
        ip += len;
        if opcode == 0x00 {
            complete = true;
            break;
        }
    }
    (out, complete)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disassembles_one_shot_shooter() {
        // bt_set(11) ; fire(1) ; se(2) ; end(1)  == the shape of ST00 enemy #14
        let mut s = vec![0x21];
        s.extend_from_slice(&[0u8; 10]); // bt_set operands
        s.push(0x20); // fire
        s.extend_from_slice(&[0x86, 0x05]); // se 5
        s.push(0x00); // end
        let (insns, complete) = disassemble(&s);
        assert!(complete);
        let names: Vec<_> = insns.iter().map(|i| i.mnemonic).collect();
        assert_eq!(names, ["bt_set", "fire", "se", "end"]);
        assert_eq!(insns[0].operands.len(), 10);
    }
}
