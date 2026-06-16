//! STD — TH04 stage data: tile-section scroll order, per-section scroll speeds,
//! enemy scripts, and the stage timeline bytecode. One `ST0n.STD` per stage,
//! stored uncompressed in the archive. Layout (ReC98 `th04/formats/std.cpp`):
//!
//! ```text
//! 0x00  u16 size          total bytes after this field (= file_len - 2)
//! 0x02  u8  map_chunk      length of the map-section-order chunk
//!       u8[map_chunk]      map_section_order: tile-section id per scrolled row
//!       u8  scroll_chunk    length of the scroll-speed chunk
//!       u8[scroll_chunk]    scroll_speeds: SubpixelLength8 per section
//!       u8  enemy_count
//!       { u8 len; u8[len] } * enemy_count    enemy scripts
//!       u8                  (one padding byte)
//!       u8[..]              stage timeline bytecode (run by `std_run`)
//! ```
//!
//! Verified against ZUN's `ST00..ST06.STD`: the timeline ends exactly at EOF.
//! The timeline + enemy-script *opcodes* (the VM) are a separate, larger
//! reverse-engineering task; this parser hands back the raw bytecode for it.

/// Hard cap from ReC98 (`STD_ENEMY_SCRIPT_COUNT`).
pub const MAX_ENEMY_SCRIPTS: usize = 32;

pub struct Std {
    /// Tile-section id for each vertically-scrolled section, in order.
    pub map_section_order: Vec<u8>,
    /// Per-section scroll speed (SubpixelLength8, 8.0 fixed → whole = `/16`).
    pub scroll_speeds: Vec<u8>,
    /// Enemy behaviour scripts (raw bytecode), up to [`MAX_ENEMY_SCRIPTS`].
    pub enemy_scripts: Vec<Vec<u8>>,
    /// Stage timeline bytecode (spawns enemies / boss over time).
    pub timeline: Vec<u8>,
}

impl Std {
    pub fn parse(d: &[u8]) -> Option<Std> {
        if d.len() < 4 {
            return None;
        }
        let size = u16::from_le_bytes([d[0], d[1]]) as usize;
        // The stored size counts everything after the u16 (the file is 2 longer).
        if size + 2 != d.len() {
            return None;
        }
        let mut off = 2usize;
        let take = |off: &mut usize, n: usize| -> Option<&[u8]> {
            let s = d.get(*off..*off + n)?;
            *off += n;
            Some(s)
        };
        let byte = |off: &mut usize| -> Option<u8> {
            let b = *d.get(*off)?;
            *off += 1;
            Some(b)
        };

        let map_chunk = byte(&mut off)? as usize;
        let map_section_order = take(&mut off, map_chunk)?.to_vec();

        let scroll_chunk = byte(&mut off)? as usize;
        let scroll_speeds = take(&mut off, scroll_chunk)?.to_vec();

        let enemy_count = byte(&mut off)? as usize;
        if enemy_count > MAX_ENEMY_SCRIPTS {
            return None;
        }
        let mut enemy_scripts = Vec::with_capacity(enemy_count);
        for _ in 0..enemy_count {
            let len = byte(&mut off)? as usize;
            enemy_scripts.push(take(&mut off, len)?.to_vec());
        }

        // One padding byte, then the rest is the stage timeline.
        let _ = byte(&mut off)?;
        let timeline = d.get(off..)?.to_vec();

        Some(Std {
            map_section_order,
            scroll_speeds,
            enemy_scripts,
            timeline,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_synthetic() {
        // size, map_chunk=2 [0,1], scroll_chunk=2 [16,16], enemy_count=1,
        // script len=2 [0xAA,0xBB], pad, timeline=[0xDE,0xAD]
        let body = [2u8, 0, 1, 2, 16, 16, 1, 2, 0xAA, 0xBB, 0x00, 0xDE, 0xAD];
        let mut file = (body.len() as u16).to_le_bytes().to_vec();
        file.extend_from_slice(&body);
        let s = Std::parse(&file).expect("parse");
        assert_eq!(s.map_section_order, [0, 1]);
        assert_eq!(s.scroll_speeds, [16, 16]);
        assert_eq!(s.enemy_scripts, vec![vec![0xAA, 0xBB]]);
        assert_eq!(s.timeline, [0xDE, 0xAD]);
    }
}
