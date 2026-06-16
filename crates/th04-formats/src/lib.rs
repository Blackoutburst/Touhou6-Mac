//! File-format parsers for Touhou 4 ~ Lotus Land Story (PC-98, 1998).
//!
//! Pure-parse, zero-dependency, mirroring the `th06` `formats` crate. Each
//! format was reverse-engineered against ZUN's original files and cross-checked
//! with the [ReC98](https://github.com/nmlgc/ReC98) source reconstruction.
//!
//! Layered like the real game: everything lives inside one packed archive
//! (the file ZUN named after the game, `東方幻想.郷`), so [`par`] is the entry
//! point — open the archive, then parse the individual members.
//!
//! Member formats (see `docs/TH4_PORTING.md` for the full inventory):
//!
//! | ext | meaning | status |
//! |-----|---------|--------|
//! | `.STD` | stage tile-order + enemy/bullet bytecode | 🔜 (gameplay core) |
//! | `.MAP` / `.MPN` | stage tilemap + tile patterns | 🔜 |
//! | `.CDG` / `.CD2` | 16-colour planar backgrounds / boss sprites | 🔜 |
//! | `.BB` / `.BFT` / `.BMT` | sprite & font tiles | 🔜 |
//! | `.M26` / `.M86` | PMD music (YM2203 / YM2608) | 🔜 |
//! | `.TXT` | dialogue (scrambled Shift-JIS) | 🔜 |

pub mod bullet;
pub mod cdg;
pub mod enemy;
pub mod enemy_vm;
pub mod math;
pub mod par;
pub mod player;
pub mod pi;
#[path = "std.rs"]
pub mod stage; // ".STD" stage data; module named `stage` to avoid shadowing `::std`
