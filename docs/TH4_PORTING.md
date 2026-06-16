# Porting Touhou 4 (Lotus Land Story) — feasibility & roadmap

> Status: **Phase 1 done** — archive format fully reverse-engineered & verified;
> `th04-formats` crate created with the (proven) unpacker.
> This document captures what we learned while deciding whether TH04 can get the
> same native-Rust + WASM treatment as TH06 in this repo.

## TL;DR

Yes, it's possible — but it is a **substantially bigger lift than the TH06 port**,
for one structural reason: TH06 is a **Windows** game whose engine is a clean
interpreter over documented data files; **TH04 is a PC-98 game** whose logic is
partly baked into 16-bit x86 machine code and whose graphics/sound assume 1998
NEC PC-9801 hardware. We do not "read data files and reimplement an interpreter"
the way TH06 did — we reimplement a small computer's worth of behaviour.

The single biggest asset that makes it tractable is **ReC98**
(<https://github.com/nmlgc/ReC98>), nmlgc's position-independent reconstruction of
the original PC-98 source. TH04 is **~46.6% reverse-engineered / ~39.3% finalized /
100% position-independent** (as of 2026-06). It is the reference, not a finished
decompilation we can mechanically translate — roughly half still needs original RE.

## Why TH06 was "easy" and TH04 is not

| | TH06 (Windows, 2002) | TH04 (PC-98, 1998) |
|---|---|---|
| Game logic | Data-driven: `.ECL`/`.STD`/`.MSG`/`.ANM` scripts run by a generic engine | Partly hardcoded in 16-bit x86; **enemy/bullet patterns are `.STD` bytecode** (good!), but much else is compiled code |
| Asset container | `PBG3` archive, community-documented | Packed/compressed blobs (`東方幻想.郷`), Shift-JIS names, format from ReC98 |
| Graphics | RGBA sprites → our wgpu renderer takes them directly | **Planar 16-colour** (4 bitplanes), GRCG/EGC HW blitter, HW text layer. Must convert planar→RGBA + emulate blit semantics |
| Sound | WAV (BGM + SFX) → rodio / Web Audio | **YM2608 FM + SSG + rhythm**, driven by PMD (`PMD.COM`). Needs an FM synth core or pre-rendered audio |
| Reference impl | `happyhavoc/th06` — complete matching decomp | `ReC98` — ~half done, Turbo C++ / x86 ASM, depends on `master.lib` |

## What's in the box (extracted from the .hdi)

The `.hdi` files are **Anex86 PC-98 hard-disk images**; `np21nt.exe` bundled
alongside is the Neko Project 21 emulator. Today TH04 "runs" by emulating an entire
PC-98 — the opposite of what we want.

Extract the real files with `tools/pc98_hdi_extract.py` (FAT12, 1024-byte sectors,
Shift-JIS names). Authentic `th04j.hdi` contents (32 files), the ones that matter:

| File | Size | What it is |
|---|---|---|
| `東方幻想.郷` | 1.05 MB | **Main packed game-data archive** (compressed; the real assets live here) |
| `幻想郷ED.DAT` | 1.12 MB | Ending data |
| `MAIN.EXE` | 156 KB | Main game — the `.STD` interpreter + hardcoded logic (LZ-packed by `ZUN.COM`) |
| `OP.EXE` | 42 KB | Opening / title |
| `PMD.COM`, `PMD86.COM`, `PMDB2.COM` | — | FM sound drivers (YM2608) |
| `ZUN.COM` | 7.7 KB | ZUN's resident decompressing EXE loader |
| `GAME.BAT` | — | Launch sequence: `zun -m / -i / -s / -o`, then PMD + `op` |
| `DISK1/`, `DISK2/` | — | Leftover install-floppy contents (`GENSO1/2.EXE` self-extractors) |

The two big data files start with high-entropy LZ/RLE data and **no plaintext TOC**,
so the archive/compression layout has to come from ReC98 / the PC-98 file-format docs.

## What we reuse from the TH06 port (verified)

`crates/engine` is **game-agnostic** and reusable nearly as-is:
- wgpu sprite renderer (640×480 logical, batched textured quads) — native Metal + WebGL2
- 60 Hz fixed-timestep loop, input mapping, offscreen screenshot path
- `audio.rs`: WAV BGM/SFX on rodio (native) + Web Audio (wasm)
- `web.rs` glue: bring-your-own-files upload, canvas, WASM entry

From `crates/formats`, only `bitstream.rs` and `lzss.rs` are generic; the rest
(`anm0`, `pbg3`, `ecl`, `std`, `msg`) are TH06-Windows-specific and do **not** apply.

The TH06 `crates/game` structure (state machine, stage loop, collision math, scoring,
deathbomb/graze) is a good **template** to mirror, but its script VMs are TH06-specific.

## What's genuinely new work (PC-98 layer)

1. **Asset archive** — reverse `東方幻想.郷` / `ED.DAT` (header + compression) → unpack to individual assets. *(Phase 1)*
2. **Planar graphics decode** — PC-98 `.PI` / `.CDG` / `.GRC` (4-plane 16-colour) → RGBA for the existing renderer. *(Phase 2)*
3. **`.STD` enemy/bullet bytecode VM** — the gameplay heart; this is the analogue of TH06's ECL and is data-driven. Cross-check opcode semantics against ReC98's TH04 sources. *(Phase 3)*
4. **Hardcoded logic** — player, bosses, scoring, stage flow that live in `MAIN.EXE` machine code: translate from ReC98 (where done) + original RE (the rest). *(Phase 4)*
5. **FM sound** — either (a) embed a YM2608 core and run the PMD song data, or (b) pre-render BGM to WAV and reuse the existing audio path. (b) is far cheaper to ship first. *(Phase 5)*

## Archive format — SOLVED (`crates/th04-formats/src/par.rs`)

`東方幻想.郷` is the TH03/04/05 variant of master.lib's PAR packfile (ReC98
`libs/master.lib/pfint21.asm`, `th03_archive_header_t`). **158 members.**

```text
header (16 bytes)
  0x00 u16 dir_size   (= 0x13e0 = 5088)
  0x02 u16 unk        (= 2)
  0x04 u16 count      (= 158)
  0x06 u16 key        (= 0x56)  directory decryption key
  0x08 u8[8] zero
directory  (dir_size bytes @ 0x10, encrypted with a rolling XOR)
  per 32-byte entry:
    0x00 u8[2] type    (0x95 0x95 = "封" → RLE-compressed; else stored)
    0x02 u8    aux
    0x03 char[13] name (8.3, NUL-padded)
    0x10 u16   packed_size
    0x12 u16   orig_size
    0x14 u32   offset  (absolute, in archive)
    0x18 u8[8] reserved
file data  (each member at its offset; RLE = ZUN's unrle scheme)
```

Directory decrypt: `al = key; for b in dir { dec = b ^ al; b = dec; al -= dec; }`
RLE: ReC98 `th01/formats/pf.cpp::unrle` (run mode after 2 equal bytes; next byte
= extra-copy count; 0 ends a run). Ported verbatim with a length bound.

**Verification:** all 158 members decode to exactly their recorded `orig_size`,
and the last member ends precisely at EOF (1,053,177 bytes). `EYE.RGB` is a valid
16-colour PC-98 palette, `ST00.STD` is plausible tile-map + bytecode.

### Asset inventory (158 files)

| ext | n | meaning | comp |
|-----|---|---------|------|
| `.STD` | 7 | **stage tile-order + enemy/bullet bytecode** (gameplay core) | raw |
| `.MAP` | 7 | stage tilemap (which tile section per row) | rle |
| `.MPN` | 8 | tile-pattern definitions | rle |
| `.CDG` | 16 | 16-colour planar backgrounds + sprites | rle |
| `.CD2` | 12 | boss sprites (`BSS*`), portraits (`KAO*`) | rle |
| `.BB`/`.BB1-9`/`.BBT` | 30 | 16×16 sprite tiles per stage | rle |
| `.BFT` | 13 | player/font sprite tiles (`MIKO*`,`MARI`,`ST*`) | rle |
| `.BMT` | 4 | more tiles | rle |
| `.M26` | 17 | PMD music, PC-9801-26 board (YM2203/OPN) | raw |
| `.M86` | 17 | PMD music, PC-9801-86 board (YM2608/OPNA) | raw |
| `.TXT` | 16 | dialogue, **separately scrambled** Shift-JIS (`_DM*`) | raw |
| `.REC` | 4 | demo replays (`DEMO1-4`) | rle |
| `.EFC`/`.EFS`/`.RGB` | 3 | effects + `EYE.RGB` palette | mixed |

Run it: `cargo run -p th04-formats --example unpack -- <東方幻想.郷> out/`

## Proposed phased roadmap

- **Phase 0 — tooling (done):** `.hdi` extractor; file inventory; ReC98 status pinned.
- **Phase 1 — unpack assets (DONE):** `東方幻想.郷` cracked + verified (158/158); `th04-formats` crate with `par` reader + `unpack` example.
- **Phase 2 — graphics decode (CDG + PI DONE):**
  - `cdg.rs` decodes `.CDG`/`.CD2` planar 16-colour images to RGBA (4 colour planes B/R/G/E + optional alpha, rows bottom-up). Verified: eyecatch (`EYE*.CDG`+`EYE.RGB`) and `BB0.CDG` character sheet.
  - `pi.rs` decodes the Yanagisawa **PI** format (`.PI`) — all full-screen art (title/opening/endings). PI embeds its own palette, decodes to chunky 4bpp, top-down. Ported from master.lib `graph_pi_load_pack.asm`. **Verified: `OP1.PI` renders as the full 東方幻想郷 ~Lotus Land Story title screen (Marisa + Reimu)**, plus ending images. Rust output matches the Python reference exactly.
  - There are **two archives**: `東方幻想.郷` (158, main game) and `幻想郷ED.DAT` (132, OP/title/menu/endings — key 0x2d). `par.rs` reads both.
  - **Open item:** CDG carries no palette; only `EYE.RGB` ships. Stage/sprite palettes live in `MAIN.EXE` or stage code (RE later). PI is unaffected (palette embedded).
  - **Title renders in-engine (DONE):** `crates/th04-game` (new bin crate) wires `th04-formats` → `th06-engine`: decode `OP1.PI` → `create_texture` → `DrawCmd` → `render_to_image`, verified offscreen (the title screen renders through the real wgpu pipeline, letterboxed in 640×480). This proves the engine integration end-to-end.
  - Still to do in Phase 2: `.BFT`/`.BB` tile sprites, `.MAP`/`.MPN` tilemaps; find stage CDG palettes.
- Remaining member formats to parse: STD (gameplay), M86 (music), TXT (dialogue).
- **Phase 2 — see something:** planar→RGBA decoder; render the title/`OP` screen in the existing wgpu window. First visible proof.
- **Phase 3 — `.STD` (parser + timeline + enemy-script disasm DONE):**
  - `stage.rs` parses `.STD` into map-section order, scroll speeds, enemy scripts (≤32), and the stage timeline. `timeline_events()` decodes the spawn schedule (reversed from `std_run` @ `th04_main.asm:16959`): `u16 frame`, `u8 count`, `count × {u8 script, i16 x, i16 y, u8 item}`; `frame==0` ends. Verified on `ST00..ST06` (ST00 = 105 frames / 246 enemies).
  - `enemy.rs` is the **enemy-script disassembler**: the full ~50-opcode VM set (movement `0x01–0x14`, bullets `0x20–0x30`, control `0x80–0x8F`, `0x00`=end/kill, `0x10`=hp/score/sprite, `0x80/81`=loop), reversed from the interpreter `sub_155DD` + jump table `off_15B4D`. Verified: every enemy script in `ST00..ST06` disassembles cleanly and terminates at `end` (51/51). e.g. ST00 #14 = `bt_set fire se end`.
  - `enemy_vm.rs` is the **stateful VM**: it executes those opcodes one frame at a time on live `Enemy` state — motion via TH04's 256-direction trig (`vx = cos8(angle)·speed >> 8`), blocking-frame timing, loops, clipping, and bullet-template/autofire state (firing bullet *objects* is the bullet system's job, next). Verified by simulation: ST00 #0 enters from the top, descends, then `set_avd` curves it right until it's clipped at the playfield edge — matching the script; ST00 #14 fires once and ends.
  - `bullet.rs` + `math.rs`: the **bullet system**. `BulletPool::spawn` fires from a `BulletTemplate` per its group (single/aimed/ring/spread/stack/random — ReC98 `types.h` semantics; `_AIMED` aims via `iatan2` at the player), and `update()` flies regular bullets straight (`vector2(angle, speed)`) and culls off-screen ones. Wired into the enemy VM: the `fire` opcode and autofire (every `autofire_interval` frames) spawn into the pool. Verified by sim: ST00 #6 spirals in, autofires a 4-way aimed spread at the player, then clips at the edge. (Special motions `BSM_*` and the decelerate ramp are TODO.)
  - **Next:** the player (`th04/main/player/` — move/shot/bomb + input), collision/scoring (reuse TH06 template), and sprite rendering (BB/BFT tiles + stage CDG backgrounds + palettes) to draw it all.
- **Phase 4 — make it a game:** player ship + shot types, bosses, scoring, stage progression, dialogue. Lean on ReC98 opcode-by-opcode where finalized.
- **Phase 5 — audio + web:** BGM (pre-rendered WAV first, FM synth later) + SFX; wire the WASM bring-your-own-`.hdi` upload (extractor logic ported to Rust/wasm).

## Suggested crate layout (mirrors TH06)

```
crates/
  engine/        # reuse as-is (maybe drop the 3D BgScene path; TH04 bg is 2D)
  formats/       # TH06 — leave alone
  game/          # TH06 — leave alone
  th04-formats/  # NEW: archive, planar gfx, .STD bytecode, PMD song parsing
  th04-game/     # NEW: state machine + stage loop (template from crates/game)
```

## Key references

- ReC98 — PC-98 source reconstruction & format notes: <https://github.com/nmlgc/ReC98>, progress: <https://rec98.nmlgc.net/>
- TH06 matching decomp (our reference model): <https://github.com/GensokyoClub/th06>
- master.lib (PC-98 HW abstraction ReC98 builds on) — needed to understand graphics/blit/sound calls.

## Honest effort estimate

TH06 here is ~7.3k lines of Rust over a year-ish of community-aligned work. TH04
adds: an undocumented-by-inspection asset format, a planar-graphics + HW-blit model,
an FM-sound story, and a reference decomp that's only ~half done. Expect **noticeably
more reverse-engineering** than TH06 required, concentrated in Phases 1, 3, 4. The
flip side: enemy patterns being `.STD` bytecode (not hand-coded asm) means the
gameplay core is more tractable than TH01/TH02 would be.
