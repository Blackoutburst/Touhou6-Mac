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
// TH04 stores the tileset in VRAM as a grid of TILE_AREA_ROWS rows × N columns
// (ReC98 th02/main/tile/tile.hpp): the precalculated tile value is
//   72 + (id / 25) * 2 + (id % 25) * 1280
// (TILE_AREA_VRAM_LEFT=72, TILE_VRAM_W=2, TILE_AREA_ROWS=25, 16*ROW_SIZE=1280).
const VRAM_LEFT: usize = 72;
const ROW_STRIDE: usize = 1280; // 16 * ROW_SIZE(80)
const TILE_VRAM_W: usize = 2; // TILE_W / 8
const AREA_ROWS: usize = 25; // RES_Y(400) / TILE_H(16)

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

    /// Tileset index for a raw tile value (inverts the VRAM-offset formula:
    /// `id = (id/25) * 25 + (id%25)`, recovering both the VRAM column and row).
    pub fn tile_index(value: u16) -> usize {
        let v = (value as usize).saturating_sub(VRAM_LEFT);
        let row = v / ROW_STRIDE; // id % AREA_ROWS
        let col = (v % ROW_STRIDE) / TILE_VRAM_W; // id / AREA_ROWS
        col * AREA_ROWS + row
    }
}
