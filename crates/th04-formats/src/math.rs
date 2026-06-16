//! TH04's fixed-point trig: 256-direction angles (u8; 0 = +x/right, 64 = +y/down)
//! and an 8.8 cos/sin table, matching master.lib `CosTable8`/`SinTable8` and
//! `vector2_near` / `iatan2`. Positions and velocities are in subpixels (16/px).

/// 8.8 cosine (`round(256·cos)`), range −256..256.
pub fn cos8(a: u8) -> i32 {
    (256.0 * (a as f64 * std::f64::consts::TAU / 256.0).cos()).round() as i32
}
/// 8.8 sine.
pub fn sin8(a: u8) -> i32 {
    (256.0 * (a as f64 * std::f64::consts::TAU / 256.0).sin()).round() as i32
}

/// Velocity (subpixels/frame) for an `angle` and `speed` (subpixels), exactly
/// as `vector2_near`: `vx = cos8(angle)·speed >> 8`.
pub fn vector2(angle: u8, speed: i32) -> (i32, i32) {
    ((cos8(angle) * speed) >> 8, (sin8(angle) * speed) >> 8)
}

/// 256-direction angle of (dx, dy), matching ReC98 `iatan2`.
pub fn iatan2(dy: i32, dx: i32) -> u8 {
    let t = ((dy as f64).atan2(dx as f64) / std::f64::consts::TAU * 256.0).round() as i32;
    (t & 0xff) as u8
}
