//! CDG / CD2 — TH04's 16-colour planar sprite & background format.
//!
//! PC-98 graphics are planar: a 16-colour image is four 1-bit bitplanes (one
//! bit of the palette index per plane), optionally preceded by a 1-bit alpha
//! mask. ReC98 `th03/formats/cdg.h` / `cdg_load.cpp`.
//!
//! On-disk layout:
//! ```text
//! 0x00  u16 bitplane_size  = (width/8) * height
//! 0x02  i16 width          (multiple of 32)
//! 0x04  i16 height
//! 0x06  i16 offset_at_bottom_left  (runtime only)
//! 0x08  u16 vram_dword_w           (runtime only, = width/32)
//! 0x0A  u8  image_count
//! 0x0B  i8  plane_layout    0 = 4 colour planes, 1 = alpha + 4 colour, 2 = alpha
//! 0x0C  u8[4] zero          (runtime segment pointers)
//! 0x10  image data          image_count × (planes × bitplane_size)
//! ```
//! Colour planes are ordered B, R, G, E (PL_B..PL_E), so the palette index of a
//! pixel is `B | R<<1 | G<<2 | E<<3`. `.CDG` files use layout 0; `.CD2` files
//! (boss sprites, portraits) use layout 1 with several images.

pub const PALETTE_LEN: usize = 16;
const HEADER_LEN: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaneLayout {
    Colors,         // 4 planes: B R G E
    ColorsAndAlpha, // 5 planes: alpha B R G E
    Alpha,          // 1 plane
}

impl PlaneLayout {
    fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::ColorsAndAlpha,
            2 => Self::Alpha,
            _ => Self::Colors,
        }
    }
    /// Total planes stored per image.
    pub fn plane_count(self) -> usize {
        match self {
            Self::Colors => 4,
            Self::ColorsAndAlpha => 5,
            Self::Alpha => 1,
        }
    }
    fn has_alpha(self) -> bool {
        matches!(self, Self::ColorsAndAlpha | Self::Alpha)
    }
}

pub struct Cdg {
    pub bitplane_size: usize,
    pub width: usize,
    pub height: usize,
    pub image_count: usize,
    pub plane_layout: PlaneLayout,
    body: Vec<u8>, // everything after the 16-byte header
}

impl Cdg {
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < HEADER_LEN {
            return None;
        }
        let bps = u16::from_le_bytes([bytes[0], bytes[1]]) as usize;
        let width = i16::from_le_bytes([bytes[2], bytes[3]]).max(0) as usize;
        let height = i16::from_le_bytes([bytes[4], bytes[5]]).max(0) as usize;
        if bps == 0 || width == 0 || width % 8 != 0 || height == 0 {
            return None;
        }
        Some(Self {
            bitplane_size: bps,
            width,
            height,
            image_count: (bytes[10] as usize).max(1),
            plane_layout: PlaneLayout::from_u8(bytes[11]),
            body: bytes[HEADER_LEN..].to_vec(),
        })
    }

    /// Bytes occupied by one image (all of its planes).
    fn image_stride(&self) -> usize {
        self.bitplane_size * self.plane_layout.plane_count()
    }

    /// Decode image `n` to RGBA8 (`width * height * 4` bytes). `palette` is the
    /// 16-colour table (see [`parse_palette`]). When the image carries an alpha
    /// plane, pixels with a clear alpha bit become fully transparent.
    pub fn decode_rgba(&self, n: usize, palette: &[[u8; 3]; PALETTE_LEN]) -> Option<Vec<u8>> {
        let stride = self.image_stride();
        let base = n.checked_mul(stride)?;
        if base + stride > self.body.len() {
            return None;
        }
        let img = &self.body[base..base + stride];
        let bps = self.bitplane_size;
        let (alpha, colors) = if self.plane_layout.has_alpha() {
            (Some(&img[..bps]), &img[bps..])
        } else {
            (None, img)
        };
        let bytes_per_row = self.width / 8;
        let mut out = vec![0u8; self.width * self.height * 4];
        for y in 0..self.height {
            for x in 0..self.width {
                let byte = y * bytes_per_row + x / 8;
                let mask = 0x80u8 >> (x % 8);
                let plane_bit = |p: usize| ((colors[p * bps + byte] & mask) != 0) as u8;
                let idx = plane_bit(0) | plane_bit(1) << 1 | plane_bit(2) << 2 | plane_bit(3) << 3;
                let [r, g, b] = palette[idx as usize];
                let a = match alpha {
                    Some(ap) => {
                        if ap[byte] & mask != 0 {
                            255
                        } else {
                            0
                        }
                    }
                    None => 255,
                };
                // CDG rows are stored bottom-to-top (master.lib blits from
                // bottom-left upward), so flip vertically into the output.
                let o = ((self.height - 1 - y) * self.width + x) * 4;
                out[o] = r;
                out[o + 1] = g;
                out[o + 2] = b;
                out[o + 3] = a;
            }
        }
        Some(out)
    }
}

/// Parse a TH04 `.RGB` palette: 16 entries, 4-bit per channel, stored G, R, B
/// (PC-98 hardware palette-register order). Expanded to 8-bit (`v * 17`).
pub fn parse_palette(rgb: &[u8]) -> [[u8; 3]; PALETTE_LEN] {
    let mut pal = [[0u8; 3]; PALETTE_LEN];
    for (i, slot) in pal.iter_mut().enumerate() {
        if i * 3 + 2 < rgb.len() {
            let g = (rgb[i * 3] & 0x0f) * 17;
            let r = (rgb[i * 3 + 1] & 0x0f) * 17;
            let b = (rgb[i * 3 + 2] & 0x0f) * 17;
            *slot = [r, g, b];
        }
    }
    pal
}
