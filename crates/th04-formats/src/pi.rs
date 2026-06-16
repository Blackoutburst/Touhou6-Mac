//! PI — the Yanagisawa "Pi" image format, used by TH04 for all full-screen art
//! (title, opening cels, endings, congratulations). Unlike CDG these embed
//! their own 16-colour palette, decode to chunky 4bpp (2 px/byte), and are
//! stored top-down.
//!
//! Ported from master.lib `graph_pi_load_pack.asm` (the loader TH04 actually
//! uses) and verified against ZUN's `OP1.PI` (the title screen renders exactly).
//!
//! The compression is Yanagisawa's: per-"previous colour" move-to-front colour
//! tables with a short prefix code, gamma-coded run lengths, and planar
//! back-references (same 2 dots / the row(s) above, optionally shifted a pixel).

/// A decoded PI image.
pub struct Pi {
    pub width: usize,
    pub height: usize,
    /// 16-entry RGB palette (PC-98 levels, already scaled toward 8-bit).
    pub palette: [[u8; 3]; 16],
    /// One palette index (0..15) per pixel, row-major, top-down.
    pub indices: Vec<u8>,
}

impl Pi {
    pub fn parse(data: &[u8]) -> Option<Pi> {
        let mut b = Bits::new(data);
        if b.byte()? != b'P' as u32 || b.byte()? != b'i' as u32 {
            return None;
        }
        while b.byte()? != 0x1A {} // comment
        while b.byte()? != 0x00 {} // dummy
        let mode = b.byte()?;
        let _aspect = (b.byte()?, b.byte()?);
        if b.byte()? != 4 {
            return None; // not 16-colour
        }
        let _machine = [b.byte()?, b.byte()?, b.byte()?, b.byte()?];
        let maexlen = (b.byte()? << 8) | b.byte()?;
        for _ in 0..maexlen {
            b.byte()?;
        }
        let width = ((b.byte()? << 8) | b.byte()?) as usize;
        let height = ((b.byte()? << 8) | b.byte()?) as usize;
        if width == 0 || width % 2 != 0 || height == 0 {
            return None;
        }
        let mut palette = [[0u8; 3]; 16];
        if mode & 0x80 == 0 {
            for slot in palette.iter_mut() {
                // PC-98 4-bit levels are stored in the high nibble (e.g. 0x70);
                // pass through, expanding any raw <16 value.
                let chan = |v: u32| -> u8 {
                    if v < 16 {
                        (v * 17) as u8
                    } else {
                        v as u8
                    }
                };
                *slot = [chan(b.byte()?), chan(b.byte()?), chan(b.byte()?)];
            }
        }

        // Per-previous-colour move-to-front tables: 16 rows of 16.
        let mut ct = [0u8; 256];
        let (mut al, mut ah, mut di) = (1u8, 0u8, 0usize);
        for _ in 0..16 {
            loop {
                al &= 15;
                ct[di] = al;
                di += 1;
                al = al.wrapping_add(1);
                ah = ah.wrapping_add(1);
                if ah & 0x0f == 0 {
                    break;
                }
            }
            al = al.wrapping_add(1);
        }

        let read_color = |b: &mut Bits, ct: &mut [u8; 256], prev: u8| -> Option<u8> {
            let (a, h) = if b.bit()? == 1 {
                (0u32, 0u32)
            } else if b.bit()? == 0 {
                (2, 0)
            } else if b.bit()? == 0 {
                (4, 1)
            } else {
                (8, 2)
            };
            let dl = b.bits(h + 1)?;
            let idx = ((a + dl) ^ 15) as usize & 0xff;
            let base = (prev as usize & 15) * 16;
            let pos = base + idx;
            let color = ct[pos];
            for k in pos..base + 15 {
                ct[k] = ct[k + 1];
            }
            ct[base + 15] = color;
            Some(color)
        };
        let read_length = |b: &mut Bits| -> Option<u64> {
            let mut cnt = 0u32;
            while b.bit()? == 1 {
                cnt += 1;
            }
            if cnt == 0 {
                Some(1)
            } else {
                Some((1u64 << cnt) | b.bits(cnt)? as u64)
            }
        };

        let bpr = width / 2; // bytes per row, 2 px each
        let total = width + bpr * height; // 2 padding rows + image
        let mut out = vec![0u8; total];
        let c1 = read_color(&mut b, &mut ct, 0)?;
        let c2 = read_color(&mut b, &mut ct, c1)?;
        let fb = ((c1 << 4) | c2) & 0xff;
        let mut di = 0usize;
        for _ in 0..width {
            out[di] = fb;
            di += 1;
        }
        let mut prev_pos: i32 = -1;
        while di < total {
            let mut p = b.bits(2)? as i32;
            if p == 3 {
                p += b.bit()? as i32;
            }
            if p == prev_pos {
                let mut last = out[di - 1] & 0x0f;
                loop {
                    let c1 = read_color(&mut b, &mut ct, last)?;
                    let c2 = read_color(&mut b, &mut ct, c1)?;
                    if di >= total {
                        break;
                    }
                    out[di] = ((c1 << 4) | c2) & 0xff;
                    di += 1;
                    last = c2;
                    if b.bit()? == 0 {
                        break;
                    }
                }
                prev_pos = -1;
                continue;
            }
            let l = read_length(&mut b)?;
            if p == 0 {
                let bb = out[di - 1];
                let dist = if (bb >> 4) == (bb & 0x0f) { 1 } else { 2 };
                let mut si = di - dist;
                for _ in 0..l {
                    if di >= total {
                        break;
                    }
                    out[di] = out[si];
                    di += 1;
                    si += 1;
                }
            } else {
                // Back-reference offset in pixels, then ReC98's _ssC byte/nibble split.
                let (mut ax, mut bh) = match p {
                    1 => (width as u32, 0u32),
                    2 => {
                        let v = (width as u32) << 1;
                        ((v & 0xffff), (v >> 16) & 1)
                    }
                    3 => (width as u32 - 1, 0),
                    _ => (width as u32 + 1, 0),
                };
                let cf0 = bh & 1;
                bh >>= 1;
                let ncf = ax & 1;
                ax = ((cf0 << 15) | (ax >> 1)) & 0xffff;
                let cf = ncf;
                let _ = bh;
                let si0 = di - ax as usize - cf as usize;
                if cf == 0 {
                    let mut si = si0;
                    for _ in 0..l {
                        if di >= total {
                            break;
                        }
                        out[di] = out[si];
                        di += 1;
                        si += 1;
                    }
                } else {
                    let mut si = si0;
                    for _ in 0..l {
                        if di >= total || si + 1 >= total {
                            break;
                        }
                        let a = out[si] as u32;
                        let sb = out[si + 1] as u32;
                        out[di] = (((a << 8) | sb) >> 4) as u8;
                        di += 1;
                        si += 1;
                    }
                }
            }
            prev_pos = p;
        }

        // Unpack the image (skip the 2 padding rows) to one index per pixel.
        let img = &out[width..width + bpr * height];
        let mut indices = vec![0u8; width * height];
        for y in 0..height {
            for x in 0..width {
                let byte = img[y * bpr + x / 2];
                indices[y * width + x] = if x % 2 == 0 { byte >> 4 } else { byte & 0x0f };
            }
        }
        Some(Pi {
            width,
            height,
            palette,
            indices,
        })
    }

    /// Expand to RGBA8 (`width * height * 4`).
    pub fn to_rgba(&self) -> Vec<u8> {
        let mut out = vec![0u8; self.width * self.height * 4];
        for (i, &idx) in self.indices.iter().enumerate() {
            let [r, g, b] = self.palette[(idx & 0x0f) as usize];
            out[i * 4] = r;
            out[i * 4 + 1] = g;
            out[i * 4 + 2] = b;
            out[i * 4 + 3] = 255;
        }
        out
    }
}

/// MSB-first bit reader over the PI byte stream. Returns `None` past EOF.
struct Bits<'a> {
    d: &'a [u8],
    pos: usize,
    cur: u32,
    nb: u32,
}

impl<'a> Bits<'a> {
    fn new(d: &'a [u8]) -> Self {
        Self { d, pos: 0, cur: 0, nb: 0 }
    }
    fn bit(&mut self) -> Option<u32> {
        if self.nb == 0 {
            self.cur = *self.d.get(self.pos)? as u32;
            self.pos += 1;
            self.nb = 8;
        }
        self.nb -= 1;
        Some((self.cur >> self.nb) & 1)
    }
    fn bits(&mut self, n: u32) -> Option<u32> {
        let mut v = 0;
        for _ in 0..n {
            v = (v << 1) | self.bit()?;
        }
        Some(v)
    }
    fn byte(&mut self) -> Option<u32> {
        self.bits(8)
    }
}
