//! MAP (`.MAP`) — TH04 stage background tile layout (ReC98 `th04/formats/map.hpp`).
//! A small header then a list of *sections*, each `ROWS_PER_SECTION` rows of
//! `TILES_MEMORY_X` tiles. The STD `map_section_order` sequences sections as the
//! stage scrolls. Each tile is a precalculated 2-byte VRAM offset; the tileset
//! index is `offset / VRAM_TILE_STRIDE` (16 rows × 80-byte VRAM rows).

pub const ROWS_PER_SECTION: usize = 5;
pub const TILES_MEMORY_X: usize = 32; // 512 / 16; only the first TILES_X are visible
pub const TILES_X: usize = 24; // visible columns (PLAYFIELD_W / 16)
const HEADER: usize = 8;
const SECTION_BYTES: usize = ROWS_PER_SECTION * TILES_MEMORY_X * 2;
const VRAM_TILE_STRIDE: usize = 1280; // 16 rows × 80 bytes/VRAM-row

pub struct Map {
    /// `sections[s][row][col]` = raw tile value (VRAM offset).
    pub sections: Vec<[[u16; TILES_MEMORY_X]; ROWS_PER_SECTION]>,
}

impl Map {
    pub fn parse(d: &[u8]) -> Option<Map> {
        if d.len() < HEADER {
            return None;
        }
        let n = (d.len() - HEADER) / SECTION_BYTES;
        let mut sections = Vec::with_capacity(n);
        for s in 0..n {
            let mut sec = [[0u16; TILES_MEMORY_X]; ROWS_PER_SECTION];
            for (r, row) in sec.iter_mut().enumerate() {
                for (c, cell) in row.iter_mut().enumerate() {
                    let o = HEADER + s * SECTION_BYTES + (r * TILES_MEMORY_X + c) * 2;
                    *cell = u16::from_le_bytes([d[o], d[o + 1]]);
                }
            }
            sections.push(sec);
        }
        Some(Map { sections })
    }

    /// Tileset index for a raw tile value.
    pub fn tile_index(value: u16) -> usize {
        value as usize / VRAM_TILE_STRIDE
    }
}
