//! BFNT (`.BFT`) — master.lib's "Bold FoNT" sprite format, used by TH04 for the
//! player and enemy sprite sheets (`MIKO*.BFT`, `MARI.BFT`, `ST0n.BFT`, …) and
//! entered into the sprite system via `super_entry_bfnt`.
//!
//! Layout (master.lib `super.inc` `bfnt_header` + `bfnt_entry_pat`):
//! ```text
//! 0x00  char id[5]   = "BFNT\x1a"
//! 0x05  u8  col       format flags
//! 0x06  u8  ver
//! 0x07  u8  x00
//! 0x08  u16 Xdots     sprite width
//! 0x0A  u16 Ydots     sprite height
//! 0x0C  u16 START     first character code
//! 0x0E  u16 END       last character code
//! 0x10  char font_name[8]
//! 0x18  u32 time
//! 0x1C  u16 extSize
//! 0x1E  u16 hdrSize
//! 0x20  u8 palette[48]   16 × RGB (8-bit, present here)
//! 0x50  patterns        (END-START+1) × (Xdots·Ydots/2) bytes
//! ```
//! Despite the planar PC-98 target, the *file* stores each pattern as **chunky
//! 4bpp** (2 pixels/byte, high nibble = left); `bfnt_entry_pat`'s B2V loop
//! transposes it to VRAM planes at load. Verified: `MARI.BFT` decodes to the
//! three Marisa-on-broom cels, `ST00.BFT` to the stage-1 enemy sprites.

pub const PALETTE_LEN: usize = 16;
const HEADER_LEN: usize = 32;
const PALETTE_LEN_BYTES: usize = 48;
const MAGIC: &[u8; 5] = b"BFNT\x1a";

pub struct Bft {
    pub width: usize,
    pub height: usize,
    pub start: u16,
    pub count: usize,
    pub palette: [[u8; 3]; PALETTE_LEN],
    data: Vec<u8>, // pattern data
    patbytes: usize,
}

impl Bft {
    pub fn parse(bytes: &[u8]) -> Option<Bft> {
        if bytes.len() < HEADER_LEN + PALETTE_LEN_BYTES || &bytes[0..5] != MAGIC {
            return None;
        }
        let width = u16::from_le_bytes([bytes[8], bytes[9]]) as usize;
        let height = u16::from_le_bytes([bytes[10], bytes[11]]) as usize;
        let start = u16::from_le_bytes([bytes[12], bytes[13]]);
        let end = u16::from_le_bytes([bytes[14], bytes[15]]);
        if width == 0 || width % 2 != 0 || height == 0 || end < start {
            return None;
        }
        // The palette is stored B, R, G per entry; master.lib's
        // bfnt_palette_set swaps it to R, G, B. Do the same here.
        let mut palette = [[0u8; 3]; PALETTE_LEN];
        for (i, c) in palette.iter_mut().enumerate() {
            let b = HEADER_LEN + i * 3;
            *c = [bytes[b + 1], bytes[b + 2], bytes[b]];
        }
        let pat_start = HEADER_LEN + PALETTE_LEN_BYTES;
        Some(Bft {
            width,
            height,
            start,
            count: (end - start + 1) as usize,
            palette,
            data: bytes[pat_start..].to_vec(),
            patbytes: width * height / 2,
        })
    }

    /// Decode pattern `n` to RGBA8. Palette index `transparent` (when given)
    /// becomes fully transparent (super-sprite clear colour; default index 0).
    pub fn decode_rgba(&self, n: usize, transparent: Option<u8>) -> Option<Vec<u8>> {
        let base = n.checked_mul(self.patbytes)?;
        if base + self.patbytes > self.data.len() {
            return None;
        }
        let pat = &self.data[base..base + self.patbytes];
        let mut out = vec![0u8; self.width * self.height * 4];
        for y in 0..self.height {
            for x in 0..self.width {
                let byte = pat[(y * self.width + x) / 2];
                let idx = if x % 2 == 0 { byte >> 4 } else { byte & 0x0f };
                let [r, g, b] = self.palette[idx as usize];
                let o = (y * self.width + x) * 4;
                out[o] = r;
                out[o + 1] = g;
                out[o + 2] = b;
                out[o + 3] = if transparent == Some(idx) { 0 } else { 255 };
            }
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_bfnt() {
        assert!(Bft::parse(&[0u8; 128]).is_none());
    }

    #[test]
    fn parses_header() {
        // minimal 8x2, 1 pattern (START=END=0): patbytes = 8*2/2 = 8
        let mut f = Vec::new();
        f.extend_from_slice(b"BFNT\x1a"); // id
        f.extend_from_slice(&[0, 22, 0]); // col, ver, x00
        f.extend_from_slice(&8u16.to_le_bytes()); // Xdots
        f.extend_from_slice(&2u16.to_le_bytes()); // Ydots
        f.extend_from_slice(&0u16.to_le_bytes()); // START
        f.extend_from_slice(&0u16.to_le_bytes()); // END
        f.extend_from_slice(&[0; 8]); // font_name
        f.extend_from_slice(&[0; 4]); // time
        f.extend_from_slice(&[0; 4]); // extSize, hdrSize
        f.extend_from_slice(&[0; 48]); // palette
        f.extend_from_slice(&[0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0]); // 8 bytes pattern
        let b = Bft::parse(&f).expect("parse");
        assert_eq!((b.width, b.height, b.count), (8, 2, 1));
        let rgba = b.decode_rgba(0, None).expect("decode");
        assert_eq!(rgba.len(), 8 * 2 * 4);
    }
}
