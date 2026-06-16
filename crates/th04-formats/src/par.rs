//! The TH04 packed archive — ZUN's `東方幻想.郷` master data file.
//!
//! This is the TH03/04/05 variant of master.lib's PAR packfile (see ReC98
//! `libs/master.lib/pfint21.asm`, struct `th03_archive_header_t`). All 158 of
//! TH04's data files live inside it. Layout:
//!
//! ```text
//! 0x00  u16 dir_size      total bytes of the (encrypted) directory
//! 0x02  u16 unk           always 2
//! 0x04  u16 count         number of entries
//! 0x06  u16 key           directory decryption key
//! 0x08  u8[8] zero
//! 0x10  directory         `dir_size` bytes, `count` × 32-byte entries, encrypted
//! ...   file data         each member at its own `offset`, optionally RLE-packed
//! ```
//!
//! The directory is obfuscated with a rolling XOR; each 32-byte entry is:
//!
//! ```text
//! 0x00  u8[2] type        0x95 0x95 ("封" in Shift-JIS) => RLE-compressed
//! 0x02  u8    aux
//! 0x03  char[13] name     8.3 filename, NUL-padded
//! 0x10  u16   packed_size compressed length within the archive
//! 0x12  u16   orig_size   decompressed length
//! 0x14  u32   offset      absolute offset of the file data in the archive
//! 0x18  u8[8] reserved
//! ```
//!
//! Verified end-to-end: all 158 members of the retail `th04j` archive decode to
//! exactly their recorded `orig_size`, and the last member ends precisely at EOF.

/// Marker in `Entry::type` (Shift-JIS "封") meaning the member is RLE-compressed.
const TYPE_COMPRESSED: [u8; 2] = [0x95, 0x95];
const HEADER_LEN: usize = 16;
const ENTRY_LEN: usize = 32;

#[derive(Debug)]
pub enum Error {
    TooShort,
    BadDirectory,
}

/// One member file inside the archive.
#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub compressed: bool,
    pub packed_size: u32,
    pub orig_size: u32,
    pub offset: u32,
}

/// A parsed TH04 archive. Owns the raw bytes; members are extracted on demand.
pub struct Archive {
    data: Vec<u8>,
    pub entries: Vec<Entry>,
    pub key: u16,
}

impl Archive {
    /// Parse the archive header + (decrypted) directory. Does not yet
    /// decompress any member.
    pub fn parse(data: Vec<u8>) -> Result<Self, Error> {
        if data.len() < HEADER_LEN {
            return Err(Error::TooShort);
        }
        let dir_size = u16::from_le_bytes([data[0], data[1]]) as usize;
        let count = u16::from_le_bytes([data[4], data[5]]) as usize;
        let key = u16::from_le_bytes([data[6], data[7]]);
        if data.len() < HEADER_LEN + dir_size || count * ENTRY_LEN > dir_size {
            return Err(Error::BadDirectory);
        }

        // Rolling-XOR decrypt of the directory (ReC98 PFSTART_decrypt loop):
        //   al = key;  for b in dir { dec = b ^ al;  al = al - dec; }
        let mut dir = data[HEADER_LEN..HEADER_LEN + dir_size].to_vec();
        let mut al = key as u8;
        for b in dir.iter_mut() {
            let dec = *b ^ al;
            *b = dec;
            al = al.wrapping_sub(dec);
        }

        let mut entries = Vec::with_capacity(count);
        for i in 0..count {
            let e = &dir[i * ENTRY_LEN..i * ENTRY_LEN + ENTRY_LEN];
            let name_end = e[3..16].iter().position(|&c| c == 0).unwrap_or(13);
            entries.push(Entry {
                name: String::from_utf8_lossy(&e[3..3 + name_end]).into_owned(),
                compressed: e[0..2] == TYPE_COMPRESSED,
                packed_size: u16::from_le_bytes([e[16], e[17]]) as u32,
                orig_size: u16::from_le_bytes([e[18], e[19]]) as u32,
                offset: u32::from_le_bytes([e[20], e[21], e[22], e[23]]),
            });
        }
        Ok(Self { data, entries, key })
    }

    /// Decompress (or copy) a member into a fresh buffer of `orig_size` bytes.
    pub fn extract(&self, entry: &Entry) -> Vec<u8> {
        let off = entry.offset as usize;
        let orig = entry.orig_size as usize;
        if entry.compressed {
            unrle(&self.data, off, orig)
        } else {
            let end = (off + orig).min(self.data.len());
            self.data[off.min(self.data.len())..end].to_vec()
        }
    }

    /// Find a member by name (case-insensitive) and extract it.
    pub fn get(&self, name: &str) -> Option<Vec<u8>> {
        self.entries
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case(name))
            .map(|e| self.extract(e))
    }
}

/// ZUN's run-length scheme (ReC98 `th01/formats/pf.cpp::unrle`). After two
/// identical bytes the stream enters "run mode": the next byte is a count of
/// additional copies, and the run continues as long as the following byte still
/// matches. A count of 0 ends a run. We stop once `orig` bytes are produced
/// (the original decompressor famously over-reads; bounding by `orig` is the
/// safe equivalent), and guard every read against the end of the archive.
fn unrle(src: &[u8], off: usize, orig: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(orig);
    let mut i = off;
    let next = |i: &mut usize| -> u8 {
        let b = src.get(*i).copied().unwrap_or(0);
        *i += 1;
        b
    };
    if i >= src.len() {
        return out;
    }
    let mut lit2 = next(&mut i);
    while out.len() < orig {
        // Literal phase: emit bytes until two equal ones appear in a row.
        loop {
            let lit1 = lit2;
            out.push(lit1);
            if out.len() >= orig {
                return out;
            }
            lit2 = next(&mut i);
            if lit1 == lit2 {
                break;
            }
        }
        let run_byte = lit2;
        out.push(run_byte); // second byte of the run
        if out.len() >= orig {
            return out;
        }
        // Run phase: `run_byte` repeats.
        loop {
            let mut runs = next(&mut i);
            while runs > 0 && out.len() < orig {
                out.push(run_byte);
                runs -= 1;
            }
            if out.len() >= orig {
                return out;
            }
            lit2 = next(&mut i);
            if lit2 != run_byte {
                break;
            }
            out.push(run_byte);
            if out.len() >= orig {
                return out;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unrle_doc_example() {
        // From ReC98's own documentation of the scheme.
        let compressed = b"zz\x02z\x01yy\x00x";
        assert_eq!(unrle(compressed, 0, 8), b"zzzzzzyy");
    }

    #[test]
    fn unrle_stops_at_orig() {
        // A long run must not exceed the requested length.
        let compressed = b"aa\xff"; // 'a','a', then 255 more
        assert_eq!(unrle(compressed, 0, 5), b"aaaaa");
    }
}
