//! TH04 game: builds GPU textures from the decoded assets and draws the live
//! `StageSim` through `th06-engine`. Used by the native binary (offscreen
//! screenshots + a windowed `play` loop) and the WASM build (`web::start_game`).

use std::collections::HashMap;

use th04_formats::bft::Bft;
use th04_formats::boss::BossKind;
use th04_formats::cdg::{self, Cdg};
use th04_formats::effects::EffectKind;
use th04_formats::map::{self, Map};
use th04_formats::mpn::Mpn;
use th04_formats::par::Archive;
use th04_formats::player::{Input, BOMB_FRAMES};
use th04_formats::sim::StageSim;
use th04_formats::stage::Std;
use th06_engine::{DrawCmd, Engine, Frame, Key, Texture};

pub mod font;
pub mod menu;

#[cfg(target_arch = "wasm32")]
pub mod web;

// Where the 384×368 playfield sits within the 640×480 frame.
const PF_LEFT: f32 = 128.0;
const PF_TOP: f32 = 40.0;
const PF_W: f32 = 384.0;
const PF_H: f32 = 368.0;

/// One player character's sprite: base texture index (neutral cel; banking
/// cels follow it), how many cels, the cel size and whether a real sprite
/// loaded (vs. a solid-colour fallback).
#[derive(Clone, Copy)]
pub struct PlayerSprite {
    base: usize,
    cels: usize,
    wh: (f32, f32),
    has: bool,
}

/// `MIKO16.BFT` — the 16×16 sheet of bullets, player shots and items (palette
/// embedded, so these render in their true colours). Indexed by cel number;
/// see [`bullet_cel`] / [`item_cel`] / [`SHOT_CEL`] for the type→cel mapping.
#[derive(Clone, Default)]
pub struct FxSheet {
    cels: Vec<(usize, f32, f32)>,
}
impl FxSheet {
    fn cel(&self, n: usize) -> Option<(usize, f32, f32)> {
        self.cels.get(n).copied()
    }
}

/// Texture indices + metadata the per-frame draw needs (the textures
/// themselves are owned by the engine once the game loop starts).
pub struct DrawData {
    /// Player sprites by character: index 0 = Reimu (`MIKO.BFT`), 1 = Marisa
    /// (`MARI.BFT`); chosen at draw time from the player's `shot_type`.
    players: [PlayerSprite; 2],
    /// Bullets / shots / items sheet (`MIKO16.BFT`).
    fx: FxSheet,
    /// `GAMEFT.BFT` glyph cels (16×16, monochrome white ink — tint at draw),
    /// indexed by cel number; see [`gameft_cel`]. Empty → fall back to the
    /// built-in 5×7 font.
    hud_font: Vec<(usize, f32, f32)>,
    /// Bomb explosion animation frames (`MIKO32.BFT` cels 0-7).
    bomb_anim: Vec<(usize, f32, f32)>,
    /// Boss body sprites by kind: the `BSS*.CD2` animation frames (texture +
    /// display size), decoded with the boss's stage `.MPN` palette; rivals reuse
    /// the player sheet (one frame). Missing/empty → fall back to a marker.
    boss_sprites: HashMap<BossKind, Vec<(usize, f32, f32)>>,
    /// Midboss body (`BSS6.CD2`) decoded with *this* stage's palette (the
    /// midboss is a per-stage placeholder); `None` → marker.
    midboss_sprite: Option<(usize, f32, f32)>,
    /// Background tiles are packed into one atlas texture so the whole
    /// background draws in a single batch (per-tile textures = hundreds of
    /// draw calls, which made WebGL drop tiles / flicker).
    tile_atlas: usize,
    tile_cols: usize,
    atlas_w: f32,
    atlas_h: f32,
    ntiles: usize,
    cel_idx: HashMap<u16, (usize, f32, f32)>,
    map: Option<Map>,
    section_order: Vec<u8>,
    /// 1-based stage number, for the "STAGE n" intro card.
    stage_no: usize,
}

/// 0-based stage index from an `STnn.STD` filename (`ST00.STD` → 0). Defaults
/// to 0 if the digits can't be parsed.
pub fn stage_index(std_name: &str) -> usize {
    std_name
        .trim_start_matches(|c: char| !c.is_ascii_digit())
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .unwrap_or(0)
}

/// Load a player character's sprite (cels 0 = neutral, 1 = lean left, 2 = lean
/// right) into `textures`, or push a single solid fallback if it's missing.
fn load_player_sprite(engine: &Engine, arc: &Archive, name: &str, fallback: [u8; 4], textures: &mut Vec<Texture>) -> PlayerSprite {
    let base = textures.len();
    match arc.get(name).and_then(|b| Bft::parse(&b)) {
        Some(b) => {
            let cels = b.count.min(3).max(1);
            for n in 0..cels {
                textures.push(engine.create_texture(&b.decode_rgba(n, Some(0)).unwrap(), b.width as u32, b.height as u32));
            }
            PlayerSprite { base, cels, wh: (b.width as f32, b.height as f32), has: true }
        }
        None => {
            textures.push(engine.create_texture(&fallback, 1, 1));
            PlayerSprite { base, cels: 1, wh: (16.0, 20.0), has: false }
        }
    }
}

/// A BFNT sprite sheet's cels appended to `textures`, registered in `cel_idx`
/// at `base + cel` (the `PAT_*` numbering from `main_pat.h`).
fn push_sheet(
    engine: &Engine,
    arc: &Archive,
    name: &str,
    base: u16,
    cel_idx: &mut HashMap<u16, (usize, f32, f32)>,
    textures: &mut Vec<Texture>,
) {
    if let Some(b) = arc.get(name).and_then(|d| Bft::parse(&d)) {
        for n in 0..b.count {
            if let Some(rgba) = b.decode_rgba(n, Some(0)) {
                let idx = textures.len();
                textures.push(engine.create_texture(&rgba, b.width as u32, b.height as u32));
                cel_idx.insert(base + n as u16, (idx, b.width as f32, b.height as f32));
            }
        }
    }
}

/// Load `MIKO16.BFT` cel-by-cel into a [`FxSheet`] (one texture per cel).
fn build_fx_sheet(engine: &Engine, arc: &Archive, textures: &mut Vec<Texture>) -> FxSheet {
    let mut cels = Vec::new();
    if let Some(b) = arc.get("MIKO16.BFT").and_then(|d| Bft::parse(&d)) {
        for n in 0..b.count {
            match b.decode_rgba(n, Some(0)) {
                Some(rgba) => {
                    let idx = textures.len();
                    textures.push(engine.create_texture(&rgba, b.width as u32, b.height as u32));
                    cels.push((idx, b.width as f32, b.height as f32));
                }
                None => cels.push((0, 16.0, 16.0)),
            }
        }
    }
    FxSheet { cels }
}

/// Decode `CD2` image 0 with a 16-colour palette into a texture.
fn decode_cd2(engine: &Engine, arc: &Archive, name: &str, pal: &[[u8; 3]; cdg::PALETTE_LEN], textures: &mut Vec<Texture>) -> Option<(usize, f32, f32)> {
    let cd = arc.get(name).and_then(|d| Cdg::parse(&d))?;
    let rgba = cd.decode_rgba(0, pal)?;
    let idx = textures.len();
    textures.push(engine.create_texture(&rgba, cd.width as u32, cd.height as u32));
    Some((idx, cd.width as f32, cd.height as f32))
}

/// Decode every animation frame of a `CD2` (idle / attack / …) into textures.
fn decode_cd2_all(engine: &Engine, arc: &Archive, name: &str, pal: &[[u8; 3]; cdg::PALETTE_LEN], textures: &mut Vec<Texture>) -> Vec<(usize, f32, f32)> {
    let Some(cd) = arc.get(name).and_then(|d| Cdg::parse(&d)) else { return Vec::new() };
    let mut frames = Vec::with_capacity(cd.image_count);
    for i in 0..cd.image_count {
        if let Some(rgba) = cd.decode_rgba(i, pal) {
            let idx = textures.len();
            textures.push(engine.create_texture(&rgba, cd.width as u32, cd.height as u32));
            frames.push((idx, cd.width as f32, cd.height as f32));
        }
    }
    frames
}

/// Decode each boss's body sprite. The stage bosses come from `BSS*.CD2`, which
/// carries no palette — but the playfield shares one palette, so decoding with
/// the boss's stage `.MPN` palette yields the right colours (verified: Orange
/// renders red-haired/green-dress with `ST00.MPN`, garish with `EYE.RGB`). The
/// stage-4 rival reuses the player sheet (`MIKO`/`MARI.BFT`).
fn build_boss_sprites(
    engine: &Engine,
    arc: &Archive,
    players: &[PlayerSprite; 2],
    textures: &mut Vec<Texture>,
) -> HashMap<BossKind, Vec<(usize, f32, f32)>> {
    let mut map = HashMap::new();
    // (kind, BSS file, the .MPN whose palette to decode it with).
    let table = [
        (BossKind::Orange, "BSS0.CD2", "ST00.MPN"),
        (BossKind::Kurumi, "BSS1.CD2", "ST01.MPN"),
        (BossKind::Elly, "BSS2.CD2", "ST02.MPN"),
        (BossKind::Yuuka, "BSS5.CD2", "ST04.MPN"),
        (BossKind::Yuuka6, "BSS5.CD2", "ST05.MPN"),
    ];
    for (kind, bss, mpn_name) in table {
        let Some(pal) = arc.get(mpn_name).and_then(|d| Mpn::parse(&d)).map(|m| m.palette) else { continue };
        let frames = decode_cd2_all(engine, arc, bss, &pal, textures);
        if !frames.is_empty() {
            map.insert(kind, frames);
        }
    }
    // Rival (stage 4) reuses the player character sheet, drawn boss-sized.
    let scale = 2.0;
    let r = &players[0];
    map.insert(BossKind::Reimu, vec![(r.base, r.wh.0 * scale, r.wh.1 * scale)]);
    let m = &players[1];
    map.insert(BossKind::Marisa, vec![(m.base, m.wh.0 * scale, m.wh.1 * scale)]);
    map
}

/// Load `GAMEFT.BFT` (the 1bpp game font) cel-by-cel for the HUD.
fn build_hud_font(engine: &Engine, arc: &Archive, textures: &mut Vec<Texture>) -> Vec<(usize, f32, f32)> {
    let mut cels = Vec::new();
    if let Some(b) = arc.get("GAMEFT.BFT").and_then(|d| Bft::parse(&d)) {
        for n in 0..b.count {
            match b.decode_rgba(n, Some(0)) {
                Some(rgba) => {
                    let idx = textures.len();
                    textures.push(engine.create_texture(&rgba, b.width as u32, b.height as u32));
                    cels.push((idx, b.width as f32, b.height as f32));
                }
                None => cels.push((0, 16.0, 16.0)),
            }
        }
    }
    cels
}

/// Load the bomb explosion animation (`MIKO32.BFT` cels 0-7).
fn build_bomb_anim(engine: &Engine, arc: &Archive, textures: &mut Vec<Texture>) -> Vec<(usize, f32, f32)> {
    let mut frames = Vec::new();
    if let Some(b) = arc.get("MIKO32.BFT").and_then(|d| Bft::parse(&d)) {
        for n in 0..b.count.min(8) {
            if let Some(rgba) = b.decode_rgba(n, Some(0)) {
                let idx = textures.len();
                textures.push(engine.create_texture(&rgba, b.width as u32, b.height as u32));
                frames.push((idx, b.width as f32, b.height as f32));
            }
        }
    }
    frames
}

/// Map a character to its `GAMEFT.BFT` cel. The font isn't plain ASCII: the
/// italic glyph block runs digits `0-9` at cels 160-169, `A-V` at 170-191 and
/// `W-Z` at 192-195. Unsupported characters (incl. space) return `None`.
fn gameft_cel(ch: char) -> Option<usize> {
    match ch.to_ascii_uppercase() {
        '0'..='9' => Some(160 + (ch as usize - '0' as usize)),
        'A'..='V' => Some(170 + (ch.to_ascii_uppercase() as usize - 'A' as usize)),
        'W'..='Z' => Some(192 + (ch.to_ascii_uppercase() as usize - 'W' as usize)),
        _ => None,
    }
}

/// Draw `text` with the GAMEFT font, top-left at (`x`, `y`), each glyph `px`
/// square, tinted `tint`. Returns the x advance (so callers can right-align).
pub(crate) fn draw_hud_text(cmds: &mut Vec<DrawCmd>, font: &[(usize, f32, f32)], x: f32, y: f32, text: &str, px: f32, tint: [f32; 4]) {
    let adv = px * 0.92; // italic glyphs overlap slightly
    let mut cx = x;
    for ch in text.chars() {
        if let Some(&(tex, ..)) = gameft_cel(ch).and_then(|c| font.get(c)) {
            cmds.push(DrawCmd { tex, dst: [cx, y, px, px], src: [0.0, 0.0, 1.0, 1.0], tint, rot: 0.0 });
        }
        cx += adv;
    }
}

/// Pixel width of `text` rendered with the GAMEFT font at glyph size `px`.
pub(crate) fn hud_text_width(text: &str, px: f32) -> f32 {
    text.chars().count() as f32 * px * 0.92
}

/// Draw `value` right-aligned ending at `x_right` with the GAMEFT font.
fn draw_hud_number(cmds: &mut Vec<DrawCmd>, font: &[(usize, f32, f32)], x_right: f32, y: f32, value: i64, px: f32, tint: [f32; 4]) {
    let s = value.max(0).to_string();
    draw_hud_text(cmds, font, x_right - s.len() as f32 * px * 0.92, y, &s, px, tint);
}

/// Map a bullet's type (`patnum`, the ported `PAT_*` ids) to a `MIKO16` cel.
/// Round-ball types render true-to-colour; the directional knife/cross types
/// fall back to a same-colour ball (orientation isn't tracked yet).
fn bullet_cel(patnum: u8) -> usize {
    match patnum {
        1 => 10, // PAT_BALL_WHITE / OUTLINED_BLUE → white ball
        2 => 28, // PAT_BALL_BLUE → blue ball
        3 => 29, // PAT_KNIFE/CROSS_YELLOW → yellow (ball stand-in)
        5 => 29, // PAT_ORB_YELLOW → yellow ball
        6 => 24, // PAT_BALL_RED → red ball
        7 => 27, // PAT_STAR → star
        8 => 30, // PAT_D_BLUE → blue teardrop
        9 => 24, // PAT_SMALL_RED → red ball
        _ => 33, // default small ball
    }
}

/// Map a dropped item's `kind` to a `MIKO16` item cel. The kinds are ReC98's
/// `item_type_t` (POWER 0, POINT 1, DREAM 2, BIGPOWER 3, BOMB 4, 1UP 5,
/// FULLPOWER 6) and the MIKO16 item icons sit in that order from cel 16.
fn item_cel(kind: u8) -> usize {
    match kind {
        0..=6 => 16 + kind as usize,
        _ => 17, // unknown → point
    }
}

/// `MIKO16` cel for the player's straight shot (white needle).
const SHOT_CEL: usize = 9;

/// A stage's background tileset (`.MPN`) packed into one atlas texture (per-tile
/// textures = hundreds of draw calls, which made WebGL drop tiles / flicker).
/// Returns `(atlas_index, atlas_w, atlas_h, ntiles)`.
fn push_tile_atlas(engine: &Engine, arc: &Archive, std_name: &str, tile_cols: usize, textures: &mut Vec<Texture>) -> (usize, f32, f32, usize) {
    let mpn = arc.get(&std_name.replace(".STD", ".MPN")).and_then(|b| Mpn::parse(&b));
    match &mpn {
        Some(m) if m.count > 0 => {
            let rows = m.count.div_ceil(tile_cols);
            let (aw, ah) = (tile_cols * 16, rows * 16);
            let mut atlas = vec![0u8; aw * ah * 4];
            for ti in 0..m.count {
                let tile = m.decode_tile(ti, None).unwrap(); // 16x16 RGBA
                let (ax, ay) = ((ti % tile_cols) * 16, (ti / tile_cols) * 16);
                for y in 0..16 {
                    let s = (y * 16) * 4;
                    let d = ((ay + y) * aw + ax) * 4;
                    atlas[d..d + 64].copy_from_slice(&tile[s..s + 64]);
                }
            }
            let idx = textures.len();
            textures.push(engine.create_texture(&atlas, aw as u32, ah as u32));
            (idx, aw as f32, ah as f32, m.count)
        }
        _ => (0, 1.0, 1.0, 0),
    }
}

/// Everything one stage needs to run + draw: its sim data and draw metadata.
pub struct StageAssets {
    pub dd: DrawData,
    pub std: Std,
    pub name: String,
}

/// Build one shared texture set covering all of `stage_names` at once. Shared
/// textures (white, both player sprites, the global `MIKO*` sheets) are created
/// once; each stage appends its own tile atlas + `ST0n.BFT` sheet. This lets the
/// menu offer any stage (practice / multi-stage) even though the engine fixes
/// the texture set up front. Returns the textures + per-stage assets in order.
pub fn build_all_stages(engine: &Engine, arc: &Archive, stage_names: &[&str]) -> (Vec<Texture>, Vec<StageAssets>) {
    let white = engine.create_texture(&[255, 255, 255, 255], 1, 1);
    let mut textures: Vec<Texture> = vec![white];

    // Both player characters, so the menu's character choice draws correctly.
    let players = [
        load_player_sprite(engine, arc, "MIKO.BFT", [255, 120, 120, 255], &mut textures), // Reimu
        load_player_sprite(engine, arc, "MARI.BFT", [102, 179, 255, 255], &mut textures), // Marisa
    ];

    // Global sprite sheets, shared by every stage (loaded once).
    let mut shared_cels: HashMap<u16, (usize, f32, f32)> = HashMap::new();
    push_sheet(engine, arc, "MIKOD.BFT", 3, &mut shared_cels, &mut textures);
    push_sheet(engine, arc, "MIKO32.BFT", 4, &mut shared_cels, &mut textures);
    push_sheet(engine, arc, "MIKO16.BFT", 38, &mut shared_cels, &mut textures);

    // Bullets / shots / items, indexed directly by MIKO16 cel number.
    let fx = build_fx_sheet(engine, arc, &mut textures);

    // The real game font (GAMEFT.BFT, 1bpp), for the HUD.
    let hud_font = build_hud_font(engine, arc, &mut textures);

    // Bomb explosion animation (MIKO32.BFT cels 0-7).
    let bomb_anim = build_bomb_anim(engine, arc, &mut textures);

    // Boss body sprites (BSS*.CD2 with each boss's stage palette; rivals reuse
    // the player sheets).
    let boss_sprites = build_boss_sprites(engine, arc, &players, &mut textures);

    let tile_cols = 16usize;
    let mut stages = Vec::with_capacity(stage_names.len());
    for &std_name in stage_names {
        let (tile_atlas, atlas_w, atlas_h, ntiles) = push_tile_atlas(engine, arc, std_name, tile_cols, &mut textures);
        let mut cel_idx = shared_cels.clone();
        push_sheet(engine, arc, &std_name.replace(".STD", ".BFT"), 128, &mut cel_idx, &mut textures);
        let map = arc.get(&std_name.replace(".STD", ".MAP")).and_then(|b| Map::parse(&b));
        // Midboss body, recoloured to this stage's palette (placeholder sprite).
        let stage_pal = arc
            .get(&std_name.replace(".STD", ".MPN"))
            .and_then(|d| Mpn::parse(&d))
            .map(|m| m.palette);
        let midboss_sprite = stage_pal.and_then(|pal| decode_cd2(engine, arc, "BSS6.CD2", &pal, &mut textures));
        let std = Std::parse(&arc.get(std_name).unwrap_or_default()).expect("parse STD");
        let section_order = std.map_section_order.clone();
        let dd = DrawData {
            players,
            fx: fx.clone(),
            hud_font: hud_font.clone(),
            bomb_anim: bomb_anim.clone(),
            boss_sprites: boss_sprites.clone(),
            midboss_sprite,
            tile_atlas,
            tile_cols,
            atlas_w,
            atlas_h,
            ntiles,
            cel_idx,
            map,
            section_order,
            stage_no: stage_index(std_name) + 1,
        };
        stages.push(StageAssets { dd, std, name: std_name.to_string() });
    }
    (textures, stages)
}

/// Build textures + draw metadata + the parsed `Std` for a single stage.
pub fn build_stage(engine: &Engine, arc: &Archive, std_name: &str) -> (Vec<Texture>, DrawData, Std) {
    let (textures, mut stages) = build_all_stages(engine, arc, &[std_name]);
    let StageAssets { dd, std, .. } = stages.remove(0);
    (textures, dd, std)
}

/// 0-based player-character index from a `shot_type` (0/1 = Reimu, 2/3 = Marisa).
fn player_char(shot_type: u8) -> usize {
    (shot_type >= 2) as usize
}

/// The end-of-stage boss for a stage + character (`STnn.STD` → stage index nn).
pub fn boss_for(std_name: &str, shot_type: u8) -> Option<BossKind> {
    BossKind::for_stage(stage_index(std_name), shot_type >= 2)
}

/// Build all textures + draw metadata + the stage sim from an archive.
pub fn setup(engine: &Engine, arc: &Archive, std_name: &str, shot_type: u8) -> (Vec<Texture>, DrawData, StageSim) {
    let (textures, dd, std) = build_stage(engine, arc, std_name);
    let sim = StageSim::new(std, shot_type, boss_for(std_name, shot_type));
    (textures, dd, sim)
}

/// The stages the menu offers, in order (`ST00` = stage 1 … `ST06` = Extra).
pub const STAGE_NAMES: [&str; 7] = [
    "ST00.STD", "ST01.STD", "ST02.STD", "ST03.STD", "ST04.STD", "ST05.STD", "ST06.STD",
];

/// Build the texture set (all stages preloaded) + a [`menu::MenuApp`] for the
/// title→menu→game flow. `title_img` is the decoded title art (RGBA, width,
/// height) — typically `OP1.PI` from `幻想郷ED.DAT`; `None` → a text-only title.
pub fn setup_menu(
    engine: &Engine,
    arc: &Archive,
    title_img: Option<(Vec<u8>, u32, u32)>,
) -> (Vec<Texture>, menu::MenuApp) {
    let (mut textures, stages) = build_all_stages(engine, arc, &STAGE_NAMES);
    let title_tex = title_img.map(|(rgba, w, h)| {
        let idx = textures.len();
        textures.push(engine.create_texture(&rgba, w, h));
        (idx, w as f32, h as f32)
    });
    let app = menu::MenuApp::new(stages, title_tex);
    (textures, app)
}

/// 3×5 bitmap digits (one byte per row, low 3 bits, MSB = left column).
const DIGITS: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111], // 0
    [0b010, 0b110, 0b010, 0b010, 0b111], // 1
    [0b111, 0b001, 0b111, 0b100, 0b111], // 2
    [0b111, 0b001, 0b111, 0b001, 0b111], // 3
    [0b101, 0b101, 0b111, 0b001, 0b001], // 4
    [0b111, 0b100, 0b111, 0b001, 0b111], // 5
    [0b111, 0b100, 0b111, 0b101, 0b111], // 6
    [0b111, 0b001, 0b001, 0b001, 0b001], // 7
    [0b111, 0b101, 0b111, 0b101, 0b111], // 8
    [0b111, 0b101, 0b111, 0b001, 0b111], // 9
];

/// Absolute-position rectangle (screen space), tex 0 tinted.
fn rect(x: f32, y: f32, w: f32, h: f32, tint: [f32; 4]) -> DrawCmd {
    DrawCmd { tex: 0, dst: [x, y, w, h], src: [0.0, 0.0, 1.0, 1.0], tint, rot: 0.0 }
}

/// Draw `value` right-aligned ending at `x_right` using the built-in 3×5 font.
fn push_number(cmds: &mut Vec<DrawCmd>, x_right: f32, y: f32, value: i64, px: f32, color: [f32; 4]) {
    let s = value.max(0).to_string();
    let advance = px * 4.0; // 3 cols + 1 gap
    let mut x = x_right - s.len() as f32 * advance;
    for ch in s.bytes() {
        let g = &DIGITS[(ch - b'0') as usize];
        for (row, bits) in g.iter().enumerate() {
            for col in 0..3 {
                if bits & (0b100 >> col) != 0 {
                    cmds.push(rect(x + col as f32 * px, y + row as f32 * px, px, px, color));
                }
            }
        }
        x += advance;
    }
}

fn solid(x: f32, y: f32, w: f32, h: f32, tint: [f32; 4]) -> DrawCmd {
    DrawCmd { tex: 0, dst: [PF_LEFT + x - w / 2.0, PF_TOP + y - h / 2.0, w, h], src: [0.0, 0.0, 1.0, 1.0], tint, rot: 0.0 }
}
fn sprite(tex: usize, x: f32, y: f32, w: f32, h: f32) -> DrawCmd {
    DrawCmd { tex, dst: [PF_LEFT + x - w / 2.0, PF_TOP + y - h / 2.0, w, h], src: [0.0, 0.0, 1.0, 1.0], tint: [1.0; 4], rot: 0.0 }
}

/// Build the draw-command list for the current sim state.
pub fn draw_frame(sim: &StageSim, dd: &DrawData) -> Vec<DrawCmd> {
    let mut cmds = Vec::new();
    // Playfield backdrop.
    cmds.push(DrawCmd { tex: 0, dst: [PF_LEFT, PF_TOP, PF_W, PF_H], src: [0.0, 0.0, 1.0, 1.0], tint: [0.04, 0.04, 0.10, 1.0], rot: 0.0 });
    // Scrolling tile background. The world scrolls *downward* (the player flies
    // forward; scenery flows toward them at the bottom). map_section_order[0]
    // is the start, shown at the bottom; later sections enter from the top.
    if let Some(mp) = &dd.map {
        let total_rows = (dd.section_order.len() * map::ROWS_PER_SECTION) as i32;
        let screen_rows = PF_H as i32 / 16; // 23 visible tile rows
        let bottom_row = sim.frame as i32 / 16; // progress, increases over time
        let frac = (sim.frame % 16) as f32; // sub-tile downward offset
        for sr in -1..=(screen_rows + 1) {
            // Screen row 0 = top = newest; bottom = oldest. Higher rows scroll
            // in from the top as `bottom_row` grows, so content moves down.
            let bg_row = bottom_row + (screen_rows - sr);
            if bg_row < 0 || bg_row >= total_rows {
                continue;
            }
            let sec = dd.section_order[bg_row as usize / map::ROWS_PER_SECTION] as usize;
            let rs = bg_row as usize % map::ROWS_PER_SECTION;
            if sec >= mp.sections.len() {
                continue;
            }
            let sy = PF_TOP + (sr * 16) as f32 + frac;
            for col in 0..map::TILES_X {
                let ti = Map::tile_index(mp.sections[sec][rs][col]);
                if ti < dd.ntiles {
                    // UV into the tile atlas (all tiles share one texture).
                    let (ax, ay) = ((ti % dd.tile_cols) * 16, (ti / dd.tile_cols) * 16);
                    let (u0, v0) = (ax as f32 / dd.atlas_w, ay as f32 / dd.atlas_h);
                    let (u1, v1) = ((ax + 16) as f32 / dd.atlas_w, (ay + 16) as f32 / dd.atlas_h);
                    cmds.push(DrawCmd {
                        tex: dd.tile_atlas,
                        dst: [PF_LEFT + (col * 16) as f32, sy, 16.0, 16.0],
                        src: [u0, v0, u1, v1],
                        tint: [1.0; 4],
                        rot: 0.0,
                    });
                }
            }
        }
    }
    // Enemies.
    for e in &sim.enemies {
        let (px, py) = (e.x as f32 / 16.0, e.y as f32 / 16.0);
        match dd.cel_idx.get(&e.anim_patnum()) {
            Some(&(idx, w, h)) => cmds.push(sprite(idx, px, py, w, h)),
            None => cmds.push(solid(px, py, 18.0, 18.0, [1.0, 0.3, 0.3, 1.0])),
        }
    }
    // Boss telegraphs (gather/circle/spark): non-damaging charge-up cues,
    // faded by age so they pulse.
    for fx in &sim.effects.effects {
        let life = 1.0 - (fx.age as f32 / fx.ttl.max(1) as f32);
        let (sz, c) = match fx.kind {
            EffectKind::Gather => (8.0 + 28.0 * (1.0 - life), [0.6, 0.9, 1.0, 0.30 * life]),
            EffectKind::CircleShrink => (8.0 + 24.0 * life, [1.0, 1.0, 1.0, 0.30 * life]),
            EffectKind::CircleGrow => (8.0 + 24.0 * (1.0 - life), [1.0, 0.9, 0.5, 0.30 * life]),
            EffectKind::Spark => (10.0 + 30.0 * (1.0 - life), [1.0, 0.7, 0.2, 0.45 * life]),
        };
        cmds.push(solid(fx.x as f32 / 16.0, fx.y as f32 / 16.0, sz, sz, c));
    }
    if let Some(b) = &sim.boss {
        if !b.defeated {
            // Spawn-rays (Kurumi): a dotted line of small blue bullets from the
            // boss to the growing tip.
            for r in b.rays.iter().filter(|r| r.flag != 0) {
                for k in 0..=6 {
                    let t = k as f32 / 6.0;
                    let x = (r.ox as f32 + (r.tx - r.ox) as f32 * t) / 16.0;
                    let y = (r.oy as f32 + (r.ty - r.oy) as f32 * t) / 16.0;
                    match dd.fx.cel(bullet_cel(2)) {
                        Some((tex, ..)) => cmds.push(sprite(tex, x, y, 10.0, 10.0)),
                        None => cmds.push(solid(x, y, 6.0, 6.0, [0.6, 0.8, 1.0, 0.9])),
                    }
                }
            }
            // Orbiting satellites: Reimu orbs (blue balls), Marisa bits (stars),
            // others (Yuuka6 chasecross) plain balls.
            let orb_cel = match b.kind() {
                BossKind::Marisa => 27, // star
                BossKind::Reimu => 28,  // blue ball
                _ => 33,                // small ball
            };
            for o in b.orbits.iter().filter(|o| o.flag != 0) {
                let (x, y) = (o.cx as f32 / 16.0, o.cy as f32 / 16.0);
                match dd.fx.cel(orb_cel) {
                    Some((tex, w, h)) => cmds.push(sprite(tex, x, y, w, h)),
                    None => cmds.push(solid(x, y, 16.0, 16.0, [0.7, 0.85, 1.0, 1.0])),
                }
            }
            // Boss body: animated BSS sprite (stage palette) if mapped, else
            // marker. Cycle the frames slowly for a living idle.
            let (bx, by) = (b.x as f32 / 16.0, b.y as f32 / 16.0);
            match dd.boss_sprites.get(&b.kind()).filter(|f| !f.is_empty()) {
                Some(frames) => {
                    let (tex, w, h) = frames[(sim.frame as usize / 24) % frames.len()];
                    cmds.push(sprite(tex, bx, by, w, h));
                }
                None => cmds.push(solid(bx, by, 56.0, 56.0, [0.85, 0.3, 0.95, 1.0])),
            }
        } else if !b.done() {
            // Defeat: a burst of explosions around the boss + an opening flash.
            let (bx, by) = (b.x as f32 / 16.0, b.y as f32 / 16.0);
            let df = b.defeat_frame();
            if df < 14 {
                let a = 0.7 * (1.0 - df as f32 / 14.0);
                cmds.push(DrawCmd { tex: 0, dst: [PF_LEFT, PF_TOP, PF_W, PF_H], src: [0.0, 0.0, 1.0, 1.0], tint: [1.0, 1.0, 1.0, a], rot: 0.0 });
            }
            if !dd.bomb_anim.is_empty() {
                let n = dd.bomb_anim.len();
                for k in 0..10usize {
                    // Puffs at fixed pseudo-random offsets, each starting at a
                    // different time then looping, so the blast stays dense
                    // across the whole defeat sequence.
                    let start = (k * 7) as u32;
                    if df < start {
                        continue;
                    }
                    let fi = ((df - start) as usize / 3) % n;
                    let ox = ((k * 37 % 100) as f32) - 50.0;
                    let oy = ((k * 53 % 88) as f32) - 44.0;
                    let (tex, w, h) = dd.bomb_anim[fi];
                    cmds.push(sprite(tex, bx + ox, by + oy, w * 1.8, h * 1.8));
                }
            }
        }
    }
    if let Some(m) = &sim.midboss {
        if !m.defeated {
            let (mx, my) = (m.x as f32 / 16.0, m.y as f32 / 16.0);
            match dd.midboss_sprite {
                // Midbosses are smaller than bosses — draw the 128px cel at ~0.7×.
                Some((tex, w, h)) => cmds.push(sprite(tex, mx, my, w * 0.7, h * 0.7)),
                None => cmds.push(solid(mx, my, 40.0, 40.0, [0.3, 0.9, 0.9, 1.0])),
            }
        }
    }
    // Enemy/boss bullets — real MIKO16 sprites (true colours), marker fallback.
    for b in &sim.bullets.bullets {
        if !b.active {
            continue;
        }
        let (x, y) = (b.x as f32 / 16.0, b.y as f32 / 16.0);
        match dd.fx.cel(bullet_cel(b.patnum)) {
            Some((tex, w, h)) => cmds.push(sprite(tex, x, y, w, h)),
            None => cmds.push(solid(x, y, 7.0, 7.0, [1.0, 1.0, 0.6, 1.0])),
        }
    }
    // Player shots.
    for s in &sim.player.shots {
        if !s.active {
            continue;
        }
        let (x, y) = (s.x as f32 / 16.0, s.y as f32 / 16.0);
        match dd.fx.cel(SHOT_CEL) {
            Some((tex, w, h)) => cmds.push(sprite(tex, x, y, w, h)),
            None => cmds.push(solid(x, y, 4.0, 10.0, [0.5, 1.0, 1.0, 1.0])),
        }
    }
    // Dropped items.
    for it in &sim.items {
        let (x, y) = (it.x as f32 / 16.0, it.y as f32 / 16.0);
        match dd.fx.cel(item_cel(it.kind)) {
            Some((tex, w, h)) => cmds.push(sprite(tex, x, y, w, h)),
            None => {
                let c = match it.kind {
                    0 => [1.0, 0.25, 0.25, 1.0],
                    1 => [0.3, 0.55, 1.0, 1.0],
                    _ => [0.95, 0.9, 0.35, 1.0],
                };
                cmds.push(solid(x, y, 10.0, 10.0, c));
            }
        }
    }

    let (ppx, ppy) = (sim.player.x as f32 / 16.0, sim.player.y as f32 / 16.0);
    // Blink while invulnerable.
    let show = sim.player.invuln == 0 || (sim.frame / 4) % 2 == 0;
    if show {
        let ps = &dd.players[player_char(sim.player.shot_type)];
        if ps.has {
            // Banking cel from the player's lean (clamped to what's available).
            let cel = match sim.player.facing.signum() {
                -1 => 1,
                1 => 2,
                _ => 0,
            }
            .min(ps.cels.saturating_sub(1));
            cmds.push(sprite(ps.base + cel, ppx, ppy, ps.wh.0, ps.wh.1));
        } else {
            cmds.push(solid(ppx, ppy, 16.0, 20.0, [0.4, 0.7, 1.0, 1.0]));
        }
    }

    // Player death explosion at the spot the player was hit.
    if sim.death_fx > 0 && !dd.bomb_anim.is_empty() {
        let prog = 1.0 - sim.death_fx as f32 / th04_formats::sim::DEATH_FX_FRAMES as f32;
        let i = ((prog * dd.bomb_anim.len() as f32) as usize).min(dd.bomb_anim.len() - 1);
        let (tex, w, h) = dd.bomb_anim[i];
        let (dx, dy) = (sim.death_pos.0 as f32 / 16.0, sim.death_pos.1 as f32 / 16.0);
        cmds.push(sprite(tex, dx, dy, w * 1.6, h * 1.6));
    }

    // Bomb: animate the MIKO32 explosion over the player while a bomb is active.
    if sim.player.bombing > 0 && !dd.bomb_anim.is_empty() {
        let prog = 1.0 - sim.player.bombing as f32 / BOMB_FRAMES as f32; // 0 → 1
        let i = ((prog * dd.bomb_anim.len() as f32) as usize).min(dd.bomb_anim.len() - 1);
        let (tex, w, h) = dd.bomb_anim[i];
        cmds.push(sprite(tex, ppx, ppy, w * 3.0, h * 3.0));
    }

    // Letterbox: hide anything drawn outside the playfield (the scrolling tiles
    // overshoot its edges; the original masks them under the HUD/border).
    let black = [0.0, 0.0, 0.0, 1.0];
    let mask = |x: f32, y: f32, w: f32, h: f32| DrawCmd { tex: 0, dst: [x, y, w, h], src: [0.0, 0.0, 1.0, 1.0], tint: black, rot: 0.0 };
    cmds.push(mask(0.0, 0.0, 640.0, PF_TOP));
    cmds.push(mask(0.0, PF_TOP + PF_H, 640.0, 480.0 - (PF_TOP + PF_H)));
    cmds.push(mask(0.0, PF_TOP, PF_LEFT, PF_H));
    cmds.push(mask(PF_LEFT + PF_W, PF_TOP, 640.0 - (PF_LEFT + PF_W), PF_H));

    // HUD in the right panel. Labels + the score use the real game font
    // (GAMEFT.BFT) when it loaded, falling back to the built-in 5×7 digits.
    // Panel inner-x ≈ 520..632.
    let panel_x = PF_LEFT + PF_W + 8.0; // 520
    let white = [1.0, 1.0, 1.0, 1.0];
    let label = [1.0, 0.85, 0.4, 1.0]; // gold
    let font = &dd.hud_font;
    if !font.is_empty() {
        draw_hud_text(&mut cmds, font, panel_x, 38.0, "SCORE", 11.0, label);
        draw_hud_number(&mut cmds, font, 632.0, 54.0, sim.score, 14.0, white);
        draw_hud_text(&mut cmds, font, panel_x, 80.0, "PLAYER", 11.0, label);
        draw_hud_text(&mut cmds, font, panel_x, 116.0, "BOMB", 11.0, label);
        draw_hud_text(&mut cmds, font, panel_x, 152.0, "POWER", 11.0, label);
    } else {
        push_number(&mut cmds, 632.0, 52.0, sim.score, 3.0, white);
    }
    // Lives (green) and bombs (blue) as icon rows, under their labels.
    for i in 0..sim.player.lives.max(0).min(8) {
        cmds.push(rect(panel_x + i as f32 * 12.0, 96.0, 9.0, 9.0, [0.4, 1.0, 0.5, 1.0]));
    }
    for i in 0..sim.player.bombs.max(0).min(8) {
        cmds.push(rect(panel_x + i as f32 * 12.0, 132.0, 9.0, 9.0, [0.5, 0.7, 1.0, 1.0]));
    }
    // Power bar (0..128).
    let pw = 104.0;
    cmds.push(rect(panel_x, 170.0, pw, 8.0, [0.2, 0.2, 0.25, 1.0]));
    let fill = pw * (sim.player.power as f32 / 128.0).min(1.0);
    cmds.push(rect(panel_x, 170.0, fill, 8.0, [1.0, 0.85, 0.3, 1.0]));

    // Boss / midboss HP bar across the top of the playfield.
    let hp = match (&sim.boss, &sim.midboss) {
        (Some(b), _) if !b.defeated => Some((b.hp, b.max_hp)),
        (_, Some(m)) if !m.defeated => Some((m.hp, 620)),
        _ => None,
    };
    if let Some((cur, max)) = hp {
        cmds.push(rect(PF_LEFT, PF_TOP + 2.0, PF_W, 5.0, [0.2, 0.05, 0.1, 1.0]));
        let w = PF_W * (cur.max(0) as f32 / max as f32);
        cmds.push(rect(PF_LEFT, PF_TOP + 2.0, w, 5.0, [1.0, 0.3, 0.4, 1.0]));
    }

    // Intro cards, centred on the playfield (drawn on top). The "STAGE n" card
    // greets the stage; the boss name card appears when the boss spawns.
    if !dd.hud_font.is_empty() {
        let cx = PF_LEFT + PF_W / 2.0;
        let centered = |cmds: &mut Vec<DrawCmd>, y: f32, s: &str, px: f32, t: [f32; 4]| {
            draw_hud_text(cmds, &dd.hud_font, cx - hud_text_width(s, px) / 2.0, y, s, px, t);
        };
        if sim.phase == th04_formats::sim::Phase::Trash && (15u16..110).contains(&sim.frame) {
            centered(&mut cmds, 180.0, &format!("STAGE {}", dd.stage_no), 30.0, [1.0; 4]);
        }
        if sim.boss_intro > 0 {
            if let Some(b) = &sim.boss {
                // Below the boss (which sits at the top) so the name stays clear.
                centered(&mut cmds, 232.0, "BOSS", 16.0, [1.0, 0.5, 0.55, 1.0]);
                centered(&mut cmds, 256.0, b.kind().name(), 26.0, [1.0, 0.8, 0.85, 1.0]);
            }
        }
    }
    cmds
}

/// Map engine input to the player's input.
pub fn map_input(inp: &th06_engine::Input) -> Input {
    Input {
        left: inp.held(Key::Left),
        right: inp.held(Key::Right),
        up: inp.held(Key::Up),
        down: inp.held(Key::Down),
        shoot: inp.held(Key::Shoot),
        focus: inp.held(Key::Focus),
        bomb: inp.pressed(Key::Bomb),
    }
}

/// The interactive update closure: step the sim from input, draw the frame.
pub fn make_update(mut sim: StageSim, dd: DrawData) -> impl FnMut(&th06_engine::Input) -> Frame {
    move |inp| {
        sim.step(&map_input(inp));
        Frame { cmds: draw_frame(&sim, &dd), bg: None, quit: false }
    }
}
