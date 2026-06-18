//! Non-damaging visual telegraphs the bosses emit — the converging "gather"
//! particles, growing/shrinking circles, and sparks the originals draw before
//! and during attacks. They carry no collision and deal no damage; a renderer
//! draws them as charge-up cues (ReC98 `gather_*` / `circles_*` / `sparks_*`).
//! Positions are subpixels (16/px).

/// Which telegraph this is. Maps to the ReC98 effect families.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectKind {
    /// `gather_add*` — particles converging on a point (attack charge-up).
    Gather,
    /// `circles_add_shrinking` — a ring collapsing inward.
    CircleShrink,
    /// `circles_add_growing` — a ring expanding outward.
    CircleGrow,
    /// `sparks_add_circle` — a burst of sparks (e.g. on a hit/defeat).
    Spark,
}

/// One live telegraph. `age`/`ttl` let the renderer fade or scale it.
#[derive(Debug, Clone, Copy)]
pub struct Effect {
    pub x: i32,
    pub y: i32,
    pub kind: EffectKind,
    pub age: u16,
    pub ttl: u16,
}

/// Collection of live telegraphs, owned by the sim and drawn by the renderer.
#[derive(Default)]
pub struct EffectPool {
    pub effects: Vec<Effect>,
}

impl EffectPool {
    pub fn new() -> Self {
        Self { effects: Vec::new() }
    }

    fn add(&mut self, x: i32, y: i32, kind: EffectKind, ttl: u16) {
        self.effects.push(Effect { x, y, kind, age: 0, ttl });
    }

    /// `gather_add` / `gather_add_only` telegraph.
    pub fn gather(&mut self, x: i32, y: i32) {
        self.add(x, y, EffectKind::Gather, 48);
    }
    /// `circles_add_shrinking`.
    pub fn circle_shrink(&mut self, x: i32, y: i32) {
        self.add(x, y, EffectKind::CircleShrink, 32);
    }
    /// `circles_add_growing`.
    pub fn circle_grow(&mut self, x: i32, y: i32) {
        self.add(x, y, EffectKind::CircleGrow, 32);
    }
    /// `sparks_add_circle`.
    pub fn spark(&mut self, x: i32, y: i32) {
        self.add(x, y, EffectKind::Spark, 24);
    }

    /// Age every telegraph and drop expired ones.
    pub fn update(&mut self) {
        for e in self.effects.iter_mut() {
            e.age = e.age.saturating_add(1);
        }
        self.effects.retain(|e| e.age < e.ttl);
    }
}
