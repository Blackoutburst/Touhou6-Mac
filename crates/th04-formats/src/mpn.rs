//! MPN (`.MPN`) — TH04's stage background tileset: a palette plus a set of
//! 16×16 planar 16-colour tiles (ReC98 `th02/formats/mpn.hpp`, loaded by
//! `mpn_load_palette_show`). The palette here is the **real stage palette**.
//!
//! Layout: 6-byte header (`"MPTN"`, `u8 count` stored minus 1, `u8 unused`),
//! then a 48-byte RGB palette, then `count+1` tiles. Each tile is 4 planes
//! (B,R,G,E), plane-separated, 16×16×1bpp = 32 bytes/plane (128 bytes/tile).

const HEADER: usize = 6;
const PALETTE_BYTES: usize = 48;
pub const TILE_W: usize = 16;
pub const TILE_H: usize = 16;
const TILE_BYTES: usize = (TILE_W / 8) * TILE_H * 4; // 128

pub struct Mpn {
    pub count: usize,
    pub palette: [[u8; 3]; 16],
    tiles: Vec<u8>,
}

impl Mpn {
    pub fn parse(d: &[u8]) -> Option<Mpn> {
        if d.len() < HEADER + PALETTE_BYTES || &d[0..4] != b"MPTN" {
            return None;
        }
        let count = d[4] as usize + 1; // stored minus 1
        let mut palette = [[0u8; 3]; 16];
        for (i, c) in palette.iter_mut().enumerate() {
            let b = HEADER + i * 3;
            *c = [d[b], d[b + 1], d[b + 2]];
        }
        let start = HEADER + PALETTE_BYTES;
        Some(Mpn {
            count,
            palette,
            tiles: d[start..].to_vec(),
        })
    }

    /// Decode tile `i` (16×16) to RGBA8. `transparent` index → alpha 0.
    pub fn decode_tile(&self, i: usize, transparent: Option<u8>) -> Option<Vec<u8>> {
        let base = i.checked_mul(TILE_BYTES)?;
        if base + TILE_BYTES > self.tiles.len() {
            return None;
        }
        let t = &self.tiles[base..base + TILE_BYTES];
        let plane = TILE_BYTES / 4; // 32
        let bpr = TILE_W / 8; // 2
        let mut out = vec![0u8; TILE_W * TILE_H * 4];
        for y in 0..TILE_H {
            for x in 0..TILE_W {
                let byte = y * bpr + x / 8;
                let mask = 0x80u8 >> (x % 8);
                let bit = |p: usize| ((t[p * plane + byte] & mask) != 0) as u8;
                let idx = bit(0) | bit(1) << 1 | bit(2) << 2 | bit(3) << 3;
                let [r, g, b] = self.palette[idx as usize];
                let o = (y * TILE_W + x) * 4;
                out[o] = r;
                out[o + 1] = g;
                out[o + 2] = b;
                out[o + 3] = if transparent == Some(idx) { 0 } else { 255 };
            }
        }
        Some(out)
    }
}
