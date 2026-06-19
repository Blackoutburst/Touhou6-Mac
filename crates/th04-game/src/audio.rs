//! BGM playback for TH04. The original music is PMD FM data (`.M26`/`.M86`);
//! this is the **pre-rendered** route — the player supplies WAV renders of each
//! track (one full loop is enough; the engine loops it). Tracks are named after
//! their archive member, lowercased with a `.wav` extension, so `ST00.M86` →
//! `st00.wav` (stage theme) and `ST00B.M86` → `st00b.wav` (boss theme); the
//! title/menu uses `title.wav`. Missing tracks are silently skipped.
//!
//! Native loads the WAVs from a `music/` folder next to the archive; the WASM
//! build receives them from the player's upload. With no audio device (or no
//! WAVs) the game just runs silent.

use th06_engine::audio::Audio;

/// Title / menu track basename.
pub const TITLE_TRACK: &str = "title.wav";

/// Stage-theme WAV name for an `STnn.STD` (e.g. `ST00.STD` → `st00.wav`).
pub fn stage_track(std_name: &str) -> String {
    std_name.to_lowercase().replace(".std", ".wav")
}

/// Boss-theme WAV name for an `STnn.STD` (e.g. `ST00.STD` → `st00b.wav`).
pub fn boss_track(std_name: &str) -> String {
    std_name.to_lowercase().replace(".std", "b.wav")
}

/// Owns the engine `Audio` (if a device + tracks are available) and swaps BGM
/// on request. `play` is idempotent — calling it every frame with the current
/// track is a no-op, so callers just declare what should be playing.
pub struct Bgm {
    audio: Option<Audio>,
}

impl Bgm {
    /// Register the given `(name, wav-bytes)` tracks. Builds no audio device if
    /// there are no tracks (stays silent).
    pub fn new(tracks: Vec<(String, Vec<u8>)>) -> Self {
        if tracks.is_empty() {
            return Self { audio: None };
        }
        let mut audio = Audio::new();
        if let Some(a) = &mut audio {
            for (name, wav) in tracks {
                a.register_bgm(&name, wav);
            }
        }
        Self { audio }
    }

    /// A silent BGM (no tracks / no device).
    pub fn silent() -> Self {
        Self { audio: None }
    }

    /// Ensure `name` is the playing track (loops it; no-op if already playing).
    pub fn play(&mut self, name: &str) {
        if let Some(a) = &mut self.audio {
            a.play_bgm(name);
        }
    }
}

/// Read every `*.wav` in `dir` into `(lowercase-basename, bytes)` pairs. Missing
/// dir → empty (no music). Native only (the web build gets WAVs from uploads).
#[cfg(not(target_arch = "wasm32"))]
pub fn load_dir(dir: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    let mut tracks = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return tracks;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("wav")) {
            if let (Some(name), Ok(bytes)) = (p.file_name().and_then(|n| n.to_str()), std::fs::read(&p)) {
                tracks.push((name.to_lowercase(), bytes));
            }
        }
    }
    tracks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_names_from_std() {
        assert_eq!(stage_track("ST00.STD"), "st00.wav");
        assert_eq!(boss_track("ST00.STD"), "st00b.wav");
        assert_eq!(stage_track("ST05.STD"), "st05.wav");
        assert_eq!(boss_track("ST05.STD"), "st05b.wav");
    }

    #[test]
    fn load_dir_reads_only_wavs() {
        let dir = std::env::temp_dir().join(format!("th04_music_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ST00.wav"), b"RIFF....WAVE").unwrap();
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        let tracks = load_dir(&dir);
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].0, "st00.wav"); // lowercased, .txt skipped
    }

    #[test]
    fn missing_dir_is_empty() {
        assert!(load_dir(std::path::Path::new("/no/such/music/dir")).is_empty());
    }
}
