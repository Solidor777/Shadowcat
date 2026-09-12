//! `playlist`/`audio-state` engine bands. `playlist` is a standard-write-rules document (owner
//! + WRITE_FIELDS, same as `table`/`note`); `audio-state` is the world's singleton transport
//! state, writable only by `WriteOrigin::AudioTransport` (the guard lives in
//! `data::sqlite::SqliteRepository::apply_intent`, mirroring the `system-defaults`/
//! `WriteOrigin::ConfigSeed` guard exactly — see that guard's own doc for why this is NOT a
//! `validate_engine`/`normalize_engine` rule).

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two crate-level deny attributes this module declares.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

/// Doc_type for a playlist: a standalone world document, standard write rules.
pub const PLAYLIST_DOC_TYPE: &str = "playlist";
/// Doc_type for the world's singleton transport-state document. Writable only by
/// `WriteOrigin::AudioTransport` — see `data::sqlite::SqliteRepository::apply_intent`'s guard.
pub const AUDIO_STATE_DOC_TYPE: &str = "audio-state";
/// `PlaylistEngine::validate`'s track-count cap.
pub const MAX_PLAYLIST_TRACKS: usize = 512;
/// `PlaylistEngine::validate`'s `fade_ms` cap (10 seconds).
pub const MAX_PLAYLIST_FADE_MS: u32 = 10_000;
/// `AudioStateEngine::validate`'s concurrently-playing-entries cap.
pub const MAX_PLAYING_TRACKS: usize = 16;

/// How a playlist advances between tracks.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::audio::PlaylistMode;
///
/// assert_ne!(PlaylistMode::Sequential, PlaylistMode::Shuffle);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(rename_all = "snake_case")]
pub enum PlaylistMode {
    /// Tracks play in authored array order, once, then stop.
    Sequential,
    /// Tracks play in an order derived from `AudioStateEngine.shuffle_seed` — deterministic so
    /// every client's "up next" display agrees.
    Shuffle,
    /// Like `Sequential`, but loops back to the first track after the last.
    LoopAll,
    /// Plays exactly one track (the first, or whichever `AudioOp::Play` names) on loop.
    Single,
}

/// A device-independent audio bus a playlist or a one-shot sound plays through. `"master"` and
/// `"ui"` are client-only mixer buses (device volume/mute only) and are never a server-side
/// channel value — a track or emitter always names one of these three.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::audio::AudioChannel;
///
/// assert_ne!(AudioChannel::Music, AudioChannel::Ambience);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(rename_all = "lowercase")]
pub enum AudioChannel {
    /// Background music.
    Music,
    /// Environmental ambience loops.
    Ambience,
    /// Sound effects, one-shots, and spatial emitters.
    Sfx,
}

/// One track in a playlist.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::audio::PlaylistTrack;
///
/// let track = PlaylistTrack { asset: "tavern-loop".into(), name: None, gain: 0.8, loop_: true };
/// assert!(track.loop_);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PlaylistTrack {
    /// Asset id of the audio to play.
    pub asset: String,
    /// Display name shown in the "now playing"/track list; `None` falls back to the asset's own
    /// filename client-side.
    #[serde(default)]
    pub name: Option<String>,
    /// Per-track gain multiplier, `0..=1` (presentation range; ingress validates finiteness).
    pub gain: f64,
    /// Loop this one track when played standalone (a playlist's own `mode` governs advancing
    /// between tracks; this flag matters only for `PlaylistMode::Single` or a direct
    /// `AudioOp::Play` naming this track without a playlist).
    #[serde(rename = "loop")]
    pub loop_: bool,
}

/// The engine body of a "playlist" document. Envelope `name` is the playlist's display name.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::audio::{AudioChannel, PlaylistEngine, PlaylistMode, PlaylistTrack};
///
/// let playlist = PlaylistEngine {
///     tracks: vec![PlaylistTrack { asset: "a1".into(), name: None, gain: 1.0, loop_: false }],
///     mode: PlaylistMode::Sequential,
///     channel: AudioChannel::Music,
///     fade_ms: 2000,
/// };
/// assert!(playlist.validate().is_ok());
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PlaylistEngine {
    /// The tracks, in authored order. Whole-array replaced on edit (`set_pointer` cannot grow
    /// arrays), same as every other engine array (`TableEngine.rows`, `NoteEngine`'s siblings).
    pub tracks: Vec<PlaylistTrack>,
    /// How the playlist advances between tracks.
    pub mode: PlaylistMode,
    /// The mixer channel every track in this playlist plays through.
    pub channel: AudioChannel,
    /// Crossfade duration (ms) between one track ending and the next starting; `0` = hard cut.
    #[serde(rename = "fadeMs")]
    pub fade_ms: u32,
}

impl PlaylistEngine {
    /// Ingress validation beyond serde shape: track count and per-field caps (`MAX_PLAYLIST_TRACKS`,
    /// non-empty asset ids, finite gains, `MAX_PLAYLIST_FADE_MS`).
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::data::engine::audio::{AudioChannel, PlaylistEngine, PlaylistMode};
    ///
    /// let empty = PlaylistEngine { tracks: vec![], mode: PlaylistMode::Sequential, channel: AudioChannel::Music, fade_ms: 0 };
    /// assert!(empty.validate().is_ok()); // an empty playlist is legal (nothing to play yet)
    /// ```
    pub fn validate(&self) -> Result<(), String> {
        if self.tracks.len() > MAX_PLAYLIST_TRACKS {
            return Err(format!(
                "a playlist may have at most {MAX_PLAYLIST_TRACKS} tracks, got {}",
                self.tracks.len()
            ));
        }
        for track in &self.tracks {
            if track.asset.is_empty() {
                return Err("every track's asset id must be non-empty".into());
            }
            if !track.gain.is_finite() {
                return Err("every track's gain must be finite".into());
            }
        }
        if self.fade_ms > MAX_PLAYLIST_FADE_MS {
            return Err(format!("fadeMs exceeds {MAX_PLAYLIST_FADE_MS}"));
        }
        Ok(())
    }
}

/// One currently-playing (or paused) entry on the world's `audio-state` singleton.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::audio::{AudioChannel, PlayingTrack};
/// use uuid::Uuid;
///
/// let entry = PlayingTrack {
///     id: Uuid::new_v4(),
///     playlist: None,
///     track_index: 0,
///     asset: "a1".into(),
///     channel: AudioChannel::Sfx,
///     gain: 1.0,
///     loop_: false,
///     started_at: 0.0,
///     paused_at: None,
/// };
/// assert!(entry.paused_at.is_none());
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PlayingTrack {
    /// Stable id for this playing entry — `AudioOp` targets it, and a `Next`/`Prev` advance
    /// assigns a FRESH id so a stale client report (naming the pre-advance id) is a no-op.
    pub id: Uuid,
    /// The source playlist, or `None` for a direct `AudioOp::Play` naming an asset with no
    /// playlist.
    #[serde(default)]
    pub playlist: Option<Uuid>,
    /// Index into the source playlist's `tracks` (meaningless when `playlist` is `None`).
    #[serde(rename = "trackIndex")]
    pub track_index: u32,
    /// Asset id currently playing (resolved from the playlist track, or given directly).
    pub asset: String,
    /// The mixer channel this entry plays through.
    pub channel: AudioChannel,
    /// Effective gain for this entry, `0..=1` (presentation range; ingress validates
    /// finiteness) — resolved from the source track's gain, or `AudioOp::Play`'s own `gain`.
    pub gain: f64,
    /// Loop this entry when it reaches its end.
    #[serde(rename = "loop")]
    pub loop_: bool,
    /// Server wall-clock ms at which this entry's position 0 was live. `f64`, not `i64`:
    /// `ws::protocol::ServerMsg::MoveStream.start_server_ms` is the existing precedent for a
    /// server-clock-ms field on a ts-rs-derived type a client reads directly — `i64`/`u64`
    /// generate as TS `bigint` (JSON has none, so `JSON.parse` always yields `number`, a
    /// silent type/runtime mismatch; see `table::RowRange`'s own doc for why THAT struct
    /// avoids `i64`).
    #[serde(rename = "startedAt")]
    pub started_at: f64,
    /// Server wall-clock ms at which this entry was paused; `Some` ⇒ paused. Same `f64`
    /// rationale as `started_at`.
    #[serde(default, rename = "pausedAt")]
    pub paused_at: Option<f64>,
}

/// The engine body of the world's singleton "audio-state" document: the server's own transport
/// writes only. Every recipient's copy is authoritative (broadcast, standard) — a joiner or
/// reconnect hears exactly what the table hears with no replay protocol, since position is
/// always derivable from `started_at`/`pausedAt` plus the calibrated server clock
/// (`WsClient.serverNow()`).
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::audio::AudioStateEngine;
///
/// let state = AudioStateEngine::default();
/// assert!(state.playing.is_empty());
/// ```
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct AudioStateEngine {
    /// Currently playing/paused entries, at most `MAX_PLAYING_TRACKS`.
    pub playing: Vec<PlayingTrack>,
    /// Deterministic shuffle order seed — every client reproduces the same "up next" order for
    /// `PlaylistMode::Shuffle` from this seed, never a client-local `Math.random()`. `u32`, not
    /// `u64`: same bigint-drift rationale as `PlayingTrack.started_at` — a 4-billion-state seed
    /// space is ample for a display-only shuffle order.
    #[serde(rename = "shuffleSeed")]
    pub shuffle_seed: u32,
}

impl AudioStateEngine {
    /// Ingress validation beyond serde shape: at most `MAX_PLAYING_TRACKS` concurrently
    /// playing entries.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::data::engine::audio::AudioStateEngine;
    ///
    /// assert!(AudioStateEngine::default().validate().is_ok());
    /// ```
    pub fn validate(&self) -> Result<(), String> {
        if self.playing.len() > MAX_PLAYING_TRACKS {
            return Err(format!(
                "at most {MAX_PLAYING_TRACKS} tracks may play at once, got {}",
                self.playing.len()
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
