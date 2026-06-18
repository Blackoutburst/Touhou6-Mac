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
  - `player.rs`: player movement (`player_move` + clamp; TH04's fixed 4px aligned / 3px diagonal — no focus slowdown) and a basic straight-up shot. Per-character shot tables (Reimu/Marisa A/B), power tiers, options, lasers and bombs are TODO.
  - `sim.rs` (`StageSim`) ties it all together into a **headless stage simulation**: the spawn timeline, the enemy VM, the bullet system and the player, plus collision (player shots → enemy damage/score; enemy bullets → player hits) and scoring. Verified: a full ST00 run spawns all 246 timeline enemies, the player shoots ~60 of them (score 780), enemies fire bullets that hit the player, and the stage reports `finished` once the timeline is exhausted and the field is clear. Collision uses approximate axis-aligned boxes; death/respawn/invulnerability and exact hitboxes are TODO.
  - **The full data + gameplay spine of TH04 now runs in Rust, headless.**
- **Phase 4 — drawing the stage (started):** `th04-game stage` wires `StageSim` into `th06-engine`: it runs the sim N frames, then draws that frame — the ST00 CDG starfield background (tiled) plus every enemy, bullet, player shot and the player — and renders it offscreen. Verified visually: a real top-down danmaku frame (enemies, their aimed/spread bullet patterns, player shots, player over the starfield). Entities are tinted markers for now; **real sprites need the BFNT (`.BFT`) format** (`super_entry_bfnt` / master.lib) — the player/enemy sprite sheets (`MIKO*.BFT`, `ST0n.BFT`); the `.BB` files are only 1-bit boss-entrance masks. Stage palettes are still placeholders (in `MAIN.EXE`/stage code).
  - `bft.rs`: the **BFNT (`.BFT`) sprite decoder** — `BFNT\x1a` header + 48-byte palette + chunky-4bpp patterns (2 px/byte; the file is chunky, `bfnt_entry_pat`'s B2V loop transposes to VRAM planes). Verified: `MARI.BFT` → Marisa-on-broom, `ST00.BFT` → the stage-1 enemy sprites. `th04-game stage` now draws **real sprites**: enemies from the stage `.BFT` (patnum → cel, approximate), the player from `MARI.BFT`, transparent index 0 — over the starfield, with bullets/shots as markers.
  - **Next:** map `patnum` → the correct global sprite sheet/cel (sheets load into cel ranges), the real stage palettes (in `MAIN.EXE`), bullet/shot sprite sheets, then the WASM build (reuse TH06 `web.rs`: canvas + archive upload) for a Chrome-playable stage. Plus polish: death/respawn/invuln, per-character player shots, bombs, music (`.M86` YM2608), dialogue (`.TXT`).
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

## Remaining work / TODO (as of 2026-06-18)

All six stages run end-to-end (native-Rust + WASM), now behind a **title/menu**
(title art → character/shot/difficulty select → play → result): real assets (PAR
archive, PI, CDG/CD2, BFNT, MPN/MAP), the enemy-script VM, bullet patterns,
player (per-character shots + bombs), lives/death/respawn, items, midboss (RE'd),
the correct **per-stage boss** (selection wired), stage clear, scrolling tile
background, sprites, and a HUD. (Stage 1 is the most polished; stages 2–6 still
need their own midbosses + palettes.)

What's left, roughly in priority order:

### Stage bosses 1–5 — DONE (ported from asm)
The per-boss C++ is *not* decompiled in ReC98 (only b6/Extra are), but ReC98 is
100% position-independent, so each boss's full per-frame update exists as x86
disassembly in `th04_main.asm`. All five are ported straight from that asm
(Normal rank; `bullet_template_tune` is a no-op for counts on Normal), living in
`crates/th04-formats/src/boss/` (one file per boss) on the shared `Boss` engine
(`boss/mod.rs`, modelled on `th04/main/boss/boss.cpp`). Each has a test that runs
it through all phases to defeat; render any with
`th04-game stage <archive> ST00.STD boss:<name> out.png`.

| Stage | Boss | asm | file | notes |
|---|---|---|---|---|
| 1 | Orange | `@orange_update$qv` 20254 (+`orange_195E4/19686/19720/197BB/19814/19878/1998B`) | `orange.rs` | 6 phases: charge → triple-ring → 4-pattern barrage → wandering spray → rotating spiral → defeat |
| 2 | Kurumi | `@kurumi_update$qv` 19251 (+`kurumi_18A79..18F8B`, `kurumi_spawnrays_add`) | `kurumi.rs` | spawn-ray engine (ray flies to edge → erupts into aimed rings), expanding 22-rings, decel rings |
| 3 | Elly | `@elly_update$qv` 24137 (+`elly_1BD4B..1C251`) | `elly.rs` | 4 HP-gated tiers cycling spreads / aimed rings / 48-ring / quad random rings |
| 4 | Reimu / Marisa | `@reimu_update$qv` 27309 / `@marisa_update$qv` 16535 (+`b4r.cpp`/`b4m.cpp`) | `rival.rs` | rival chosen by playchar; 8 orbs / 4 bits spin out + orbit; tiered patterns |
| 5 | Yuuka | `@yuuka5_update$qv` 14335 (+`yuuka5_15F97..16389`, `b6.cpp`) | `yuuka.rs` | chasecross sweep + spin rings + safety bounce, final **Master Spark** tier |

Deviations (feel-preserving, see `boss/mod.rs` docs): the exact `randring2`
sequence is replaced by a local PRNG; cosmetic telegraphs (gather/circle/spark)
are surfaced as non-damaging `effects` markers (now rendered).

### Exact-fidelity pass (in progress)
Upgrading each boss from faithful-structure to opcode-exact:
- **Bullet physics — DONE** (`bullet.rs`): the real slow-bullet decelerate ramp
  (`BMF_DECELERATE`: <4px bullets start at 4.5px and ramp down over 32f) and all
  special motions (`Bsm`: `SPEEDUP`, `DECEL_THEN_TURN[_AIMED]`, `DECEL_TO_ANGLE`,
  `BOUNCE_*`, `GRAVITY`), from `bullet/update.cpp`. Applies to every boss + trash.
- **Elly — EXACT** (`boss/elly.rs`): figure-8 orbit (`elly_1BC73`), 5 HP tiers
  with the real per-tier mode windows, all 10 modes, the 48-ring finale. Only the
  `byte_25A26` aim-anim counter (from the 290-line `elly_1B95C`) is approximated
  by a frame countdown.
- **Reimu — EXACT** (`boss/reimu.rs`): the orb engine (spin-out→fly+bounce+
  gravity, `reimu_1EBF3`/`orbs_add_*`), all 13 phases (attacks 2/4/6/8/9 at gates
  7900/6300/4500/2700/900/0, moves 3/5/7/10, final 11), jink movement, every mode
  (`reimu_1ED15`..`1F17C`). A few `boss_statebyte` setup values use Normal-rank
  constants.
- **Marisa — EXACT** (`boss/marisa.rs`): the four destructible **bits** (hp
  220/400/280/450) that spin out to a 64px orbit and act as armour
  (`marisa_179BC` divides damage by `bits_alive+1`), `flystep_pointreflected`
  flight, the `0xFF` wander-selector driving the mode cycle, and all 10 modes
  (`marisa_16DFF`..`17813`). Bits are shoot-down-able and solid (sim handles
  shot/player collision vs `boss.orbits`). A couple of `boss_statebyte` setup
  values use Normal-rank constants.
- **Yuuka — EXACT** (`boss/yuuka.rs`): all 19 phases (intro/settle, three
  attack-A/glide/safety-circle cycles, attack-B, Master Spark ×2, final, defeat)
  with the exact HP arithmetic, the `15ECE` inter-pattern glide, chasecross
  sweep, spin-rings + SPEEDUP cross bursts, bouncing safety-circle crosses,
  accelerating aimed ring, narrowing spread, and the final symmetric spreads.
  The Master Spark's rotating ring + random fill are exact; its **thick laser**
  is approximated as a dense fast column + a `circle_grow` telegraph.
- **Kurumi/Orange — tightened**: Kurumi's spawn-ray bursts now use `Bsm::Speedup`
  and its phase-3 rings `Bsm::DecelThenTurn` (±0x40, exact); Orange's sub-4px
  bullets ride the real decelerate ramp automatically. **All 5 stage bosses are
  now exact.**
- **Stage 6 Yuuka — DONE** (`boss/yuuka6.rs`): `b6.cpp` turned out to be only
  struct/anim declarations — the logic is asm (`@yuuka6_update$qv` 22545 + ~30
  subs), the largest boss. All 18 phases + exact HP arithmetic (13300 → 10600 →
  7600 → 5400 → 3400 → 1200 → 0), parasol vanish/appear, the **mirror point**
  (twin attacks from Yuuka + her reflection), homing **chasecross** bullets
  (reusing the satellite pool; sim resolves their collision), the bouncing
  decel-turn cross rings, gravity randoms, mirrored spreads, sweep, rotating
  rings, and the finale. Faithful-structure rather than every-sub-exact (it is
  ~2× any other boss): the safety-circle is a shrinking telegraph + aimed-ring
  fire, the thick laser a fast column, and the parasol-shield damage-redirect is
  folded into the normal hittest. **This completes the entire stages 1-6 boss
  roster.**

**Boss roster status:** stages 1-5 fully exact, stage 6 faithful-structure +
signature mechanics. Bullet physics (decelerate ramp + all `BSM_*` special
motions) exact across all of them. 30 `th04-formats` tests pass; render any with
`th04-game stage <archive> ST00.STD boss:<orange|kurumi|elly|reimu|marisa|yuuka|yuuka6>`.

**Per-stage boss selection — DONE.** `BossKind::for_stage(stage, playing_marisa)`
maps `STnn.STD` → the stage's boss (stage 4 = the rival, chosen by playchar:
Reimu's player faces Marisa, Marisa's faces Reimu; `ST06`/Extra = no roster
boss). `StageSim::new(std, shot_type, boss_kind)` carries it and spawns
`Boss::from_kind` at the boss phase instead of the old hardcoded `Boss::orange()`.
`th04-game setup` derives it from the STD name + character, so
`th04-game play <archive> STnn.STD` now reaches the right boss for any stage, and
`th04-game stage … boss` (no name) renders that stage's own boss. Verified on the
real archive: ST00→Orange(3050), ST01→Kurumi(4800), ST02→Elly(6000),
ST03→rival(6000), ST04→Yuuka(9000), ST05→Yuuka6(13300). Stages 2–6 timelines
already parse/run, so all stages are now reachable end-to-end.

**Remaining boss work:** midbosses 1–3 are asm-only (`@midbossN_update$qv`);
only midboss 1 is RE'd, and `MIDBOSS_FRAME` is still a placeholder for all stages
(the boss phase is gated behind a midboss interlude that currently always spawns
Midboss 1 at frame 2400 regardless of stage).
- **Exact player shot tables** — Reimu/Marisa A/B × 10 power levels + Marisa A's
  option lasers (the ~40 `shot_*` functions in `th04_main.asm`). Currently a
  simplified fan/column model.
- **Item drop table** — the real per-enemy drops (`th04/main/item/enemy_drops`)
  and item kinds/values; currently every kill drops a point item.
- **Exact hitboxes** + deathbomb window; **power items** raising shot level.
- **Midboss activation frame** — `MIDBOSS_FRAME` is a placeholder; find the real
  per-stage `frames_until`.
- **Banking-cel order** — confirm MARI.BFT cel 1/2 = left/right (currently guessed).

### Graphics polish
- **Bullet / shot / item sprites — DONE.** `MIKO16.BFT` (the 16×16 sheet of
  bullets, the player's needle shot and the items — palette embedded, so true
  colours) is loaded cel-by-cel into a `FxSheet` (`build_fx_sheet`); `draw_frame`
  renders enemy/boss bullets via `bullet_cel(patnum)` (the ported `PAT_*` ids →
  MIKO16 cels), player shots via `SHOT_CEL`, and dropped items via
  `item_cel(kind)`, all falling back to the old markers if the sheet is missing.
  Verified: Orange fires white balls, the player streams needle shots, stage
  trash + items render true-to-colour. Limitations: directional knife/cross
  types (Elly/Yuuka) fall back to a same-colour ball (bullet orientation isn't
  tracked yet); MIKO32 big bullets / options aren't wired. Inspect any sheet
  with `th04-game sheet <archive> <NAME.BFT> [out.png] [scale] [cols]`.
- **Backgrounds + boss palettes — SOLVED (no `MAIN.EXE` RE needed).** MPN tiles
  carry their own palette, and the PC-98 playfield shares one 16-colour palette,
  so the **boss `CD2` sprites decode correctly with their stage's `.MPN`
  palette** — verified: Orange is red-haired/green-dress with `ST00.MPN`, garish
  with `EYE.RGB`. (The doc's old "palettes live in MAIN.EXE" worry was moot — the
  MPN palette *is* the playfield palette.)
- **Boss sprites — DONE.** `build_boss_sprites` decodes each stage boss from
  `BSS*.CD2` with its stage `.MPN` palette into `DrawData::boss_sprites`;
  `draw_frame` draws `sim.boss` by `Boss::kind()`, marker fallback. Mapping:
  Orange→BSS0, Kurumi→BSS1, Elly→BSS2, Yuuka→BSS5(ST04), Yuuka6→BSS5(ST05); the
  stage-4 rival reuses the player sheet (Reimu=`MIKO.BFT`, Marisa=`MARI.BFT`,
  2×). Verified in-engine for all. Inspect any CD2 with
  `th04-game cd2 <archive> <NAME.CD2> <PAL.MPN|.RGB> [out.png]`.
- **Real HUD font — DONE.** `Bft` now has a **1bpp branch** (detected as "too
  small to be 4bpp" — monochrome cels, no palette, ink → opaque white for
  tinting), so `GAMEFT.BFT` decodes. It isn't plain ASCII: the italic glyph
  block runs `0-9` at cels 160-169, `A-V` at 170-191, `W-Z` at 192-195
  (`gameft_cel`). The HUD score + the `SCORE/PLAYER/BOMB/POWER` labels now draw
  in the real game font (`build_hud_font`/`draw_hud_text`), 5×7 fallback if the
  font is absent. The **menu** now draws in the same GAMEFT font too
  (`menu.rs` `gt`/`gtc`/`gw` wrappers over the shared `hud_font`, 5×7 fallback),
  so the title/menu/result text matches the HUD. (The original side-panel border
  art is still TODO.)
- **Boss animation — DONE.** `BSS*.CD2`'s frames all decode (`decode_cd2_all`);
  the boss body cycles them slowly for a living idle (`frame/24 % nframes`).
- **Midboss + orbs/rays — DONE.** The midboss body draws from `BSS6.CD2`
  recoloured to the current stage's palette (`DrawData::midboss_sprite`, decoded
  per-stage at 0.7×; a placeholder until per-stage midbosses are identified).
  Boss satellites use MIKO16 cels by kind — Reimu orbs = blue balls, Marisa bits
  = stars, Yuuka6 chasecross = balls — and Kurumi's spawn-rays draw as a dotted
  line of small blue bullets. Verified in-engine (Reimu's blue-ball orb ring, the
  midboss maid firing). Boss **portraits** (`KAO*.CD2`, decode fine with the
  stage palette) are unused pending the dialogue system.
- **Still markers (intentional):** the **telegraph effects** (gather/circle/spark
  charge-up glows) stay as translucent markers — they're cosmetic cues, not
  sprites.
- **Tile-atlas linear-filter seams** — switch the atlas to nearest filtering if
  seams show when upscaled.

### Content / systems
- **Stages 2–6** — reachable end-to-end: `ST01..ST06` timelines parse/run and
  each spawns its correct boss (per-stage selection wired). Still per-stage:
  the real midboss-activation frames (placeholder), the per-stage midbosses 2–6
  (asm-only), and the stage palettes.
- **Dialogue** — `.TXT` (scrambled Shift-JIS) + the dialog system.
- **Music** — `.M26`/`.M86` PMD songs (YM2203/YM2608); needs an FM synth core or
  pre-rendered audio, then wire into `th06-engine`'s audio.
- **Title/menu — DONE.** `menu.rs` is a state machine driven by the same 60 Hz
  update closure: title (real `OP1.PI` art from `幻想郷ED.DAT`, with the session
  hi-score) → main menu → character (Reimu/Marisa) → shot type (A/B) → difficulty
  → play → result (`STAGE CLEAR` / `ALL CLEAR` / `GAME OVER` + score & hi-score)
  → title. Selections build the `StageSim` on confirm (`Std` is `Clone`d,
  character → `shot_type` → player sprite + stage-4 rival). Both player sprites
  preload (`MARI.BFT`/`MIKO.BFT`). A built-in 5×7 font (`font.rs`) renders the
  text (the authentic `GAMEFT.BFT` is still a TODO).
  - **All stages preload** ([`build_all_stages`]): one shared texture set covers
    every stage's tiles + sprites up front (the engine fixes textures for the
    whole loop), so the menu can launch any stage.
  - **START** plays the full game — clearing a stage advances to the next,
    carrying score / lives / bombs / power (`StageSim::restore`) — through stage 6
    to `ALL CLEAR`. **PRACTICE START** → a stage picker (stages 1–6). **EXTRA
    START** → `ST06`. `MUSIC ROOM` / `OPTION` are shown but disabled.
  - **Scoring / extends:** score accumulates across chained stages; extra lives
    are granted at `EXTEND_SCORES` milestones (`StageSim::award_extends`; the
    exact ReC98 thresholds are a TODO — current values are tuned to this port's
    simplified scoring). A session hi-score is shown on the title/menu/result.
  - **Difficulty tunes bullet counts** (`bullet.rs` `tuned_count` +
    `BulletPool::set_rank`, threaded via `StageSim::set_rank` ← the menu's rank):
    multi-bullet patterns (rings/spreads/stacks/random) scale ×3⁄4 (Easy) / ×1
    (Normal, no-op — the rank the patterns are authored at) / ×5⁄4 (Hard) / ×3⁄2
    (Lunatic); singles never scale. Verified on Orange: 36 bullets (Easy) vs 72
    (Lunatic) at the same frame. The exact ReC98 per-pattern deltas are a TODO —
    this is a documented proportional approximation.
  - **OPTION screen** (`Screen::Option`): edits START LIVES (1–5) and START
    BOMBS (0–3) with Left/Right; the values seed every fresh run (chained stages
    carry over instead via `restore`). `MUSIC ROOM` stays disabled (no audio).
  - Entry points: `th04-game menu <archive>` (windowed) and the WASM build (which
    also opens on the menu); offscreen check: `th04-game menushot <archive>`.
    Difficulty is exercisable offscreen too:
    `th04-game stage <archive> ST00.STD boss out.png <easy|normal|hard|lunatic>`.
  - **Still TODO: replays** (deprioritized), per-stage progress save, the real
    `GAMEFT.BFT` font, a fuller `OPTION` (key config / volume once audio exists),
    and the exact ReC98 per-pattern rank deltas.

### Build / infra
- WASM bundle built locally to `web/pkg-th04/` (gitignored). `wasm-pack` is
  installed. The WASM build now opens on the title/menu. Native: windowed
  title→menu→play `cargo run -p th04-game -- menu <archive>`, or jump straight
  to a stage `cargo run -p th04-game -- play <archive> ST00.STD`.
