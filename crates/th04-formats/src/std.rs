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

/// One enemy spawn from the stage timeline — `enemies_add(script, x, y, arg)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spawn {
    /// Index into [`Std::enemy_scripts`].
    pub script_index: u8,
    /// Spawn position (subpixel, signed; e.g. y = -256 starts above the screen).
    pub x: i16,
    pub y: i16,
    pub arg: u8,
}

/// All spawns that fire on a given stage frame.
#[derive(Debug, Clone)]
pub struct TimelineFrame {
    pub frame: u16,
    pub spawns: Vec<Spawn>,
}

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

    /// Decode the stage timeline into per-frame spawn lists (ReC98 `std_run`).
    /// Each record is `u16 frame`, `u8 count`, then `count` × 8-byte spawn
    /// entries; a `frame` of 0 terminates the timeline.
    pub fn timeline_events(&self) -> Vec<TimelineFrame> {
        let tl = &self.timeline;
        let mut out = Vec::new();
        let mut i = 0usize;
        while i + 2 <= tl.len() {
            let frame = u16::from_le_bytes([tl[i], tl[i + 1]]);
            if frame == 0 {
                break;
            }
            i += 2;
            let count = match tl.get(i) {
                Some(&c) => c as usize,
                None => break,
            };
            i += 1;
            let mut spawns = Vec::with_capacity(count);
            for _ in 0..count {
                if i + 8 > tl.len() {
                    break;
                }
                spawns.push(Spawn {
                    script_index: tl[i],
                    x: i16::from_le_bytes([tl[i + 1], tl[i + 2]]),
                    y: i16::from_le_bytes([tl[i + 3], tl[i + 4]]),
                    arg: tl[i + 5],
                });
                i += 8; // 6 read + 2 padding
            }
            out.push(TimelineFrame { frame, spawns });
        }
        out
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

    #[test]
    fn timeline_decodes() {
        let mut tl = Vec::new();
        tl.extend_from_slice(&100u16.to_le_bytes()); // frame 100
        tl.push(1); // 1 spawn
        tl.push(3); // script #3
        tl.extend_from_slice(&50i16.to_le_bytes()); // x
        tl.extend_from_slice(&(-256i16).to_le_bytes()); // y (above screen)
        tl.push(7); // arg
        tl.extend_from_slice(&[0, 0]); // padding
        tl.extend_from_slice(&0u16.to_le_bytes()); // terminator
        let s = Std {
            map_section_order: vec![],
            scroll_speeds: vec![],
            enemy_scripts: vec![],
            timeline: tl,
        };
        let ev = s.timeline_events();
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].frame, 100);
        assert_eq!(
            ev[0].spawns,
            vec![Spawn { script_index: 3, x: 50, y: -256, arg: 7 }]
        );
    }
}
