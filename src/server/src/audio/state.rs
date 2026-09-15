//! The pure `audio-state` transport engine: every `AudioOp` reduces to a fresh
//! `AudioStateEngine`, deterministically, with no I/O — the same posture
//! `combat::transition`'s pure functions take, so it runs identically under
//! `apply_intent` and any future replay/simulation path. `AudioOp::Next`'s
//! elapsed-duration gate is NOT here (see `transport::handle_transport`'s own
//! doc for why) — this function performs the advance unconditionally once
//! called.

use std::fmt;

use uuid::Uuid;

use crate::data::engine::{
    AudioChannel, AudioStateEngine, PlayingTrack, PlaylistEngine, PlaylistMode, MAX_PLAYING_TRACKS,
};
use crate::ws::protocol::AudioOp;

/// Why an `AudioOp` was refused. Player-presentable via `Display`.
///
/// # Examples
///
/// ```
/// use shadowcat::audio::state::AudioError;
///
/// assert_eq!(AudioError::UnknownId.to_string(), "no such playing track");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioError {
    /// The op named a `PlayingTrack.id` that does not exist in the current state (already
    /// stopped, already advanced past, or never existed).
    UnknownId,
    /// `AudioOp::Play` named a `playlist` id the caller could not resolve.
    UnknownPlaylist,
    /// The playlist named by `AudioOp::Play` has no tracks to play.
    EmptyPlaylist,
    /// The resulting `playing` set would exceed `MAX_PLAYING_TRACKS`.
    Cap,
    /// A gain value was non-finite.
    InvalidGain,
}

impl fmt::Display for AudioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            AudioError::UnknownId => "no such playing track",
            AudioError::UnknownPlaylist => "no such playlist",
            AudioError::EmptyPlaylist => "that playlist has no tracks",
            AudioError::Cap => "too many tracks are already playing",
            AudioError::InvalidGain => "gain must be a finite number",
        })
    }
}

/// Advance `seed` one step — the LCG `AudioOp::Play` applies to `AudioStateEngine.shuffle_seed`
/// on a fresh Shuffle play, so a world's shuffle order does not repeat identically forever.
/// The seed is still the only input to the order itself, so every client reproduces the
/// current walk from the committed seed exactly.
///
/// # Examples
///
/// ```
/// use shadowcat::audio::state::advance_seed;
///
/// assert_ne!(advance_seed(0), 0);
/// assert_eq!(advance_seed(advance_seed(7)), advance_seed(advance_seed(7)));
/// ```
pub fn advance_seed(seed: u32) -> u32 {
    seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223)
}

/// The seeded shuffle order over `len` track indices: a Fisher–Yates permutation driven by a
/// splitmix32 stream seeded from `shuffle_seed` — deterministic and reproducible by every
/// client from the same seed, never a per-client `Math.random()`, and never the degenerate
/// rotation (seed + ordinal) % len produces (it short-cycles when gcd(seed+1, len) > 1).
///
/// # Examples
///
/// ```
/// use shadowcat::audio::state::seeded_permutation;
///
/// let perm = seeded_permutation(4, 42);
/// let mut sorted = perm.clone();
/// sorted.sort_unstable();
/// assert_eq!(sorted, vec![0, 1, 2, 3]);
/// assert_eq!(seeded_permutation(4, 42), perm); // deterministic
/// ```
pub fn seeded_permutation(len: u32, shuffle_seed: u32) -> Vec<u32> {
    let mut perm: Vec<u32> = (0..len).collect();
    let mut state = shuffle_seed;
    for i in (1..len).rev() {
        // splitmix32 step: a full-period 32-bit generator, so no seed sticks the walk.
        state = state.wrapping_add(0x9E37_79B9);
        let mut z = state;
        z = (z ^ (z >> 16)).wrapping_mul(0x21F0_AAAD);
        z = (z ^ (z >> 15)).wrapping_mul(0x735A_2D97);
        z ^= z >> 15;
        let j = (z as u64 % (i as u64 + 1)) as u32;
        perm.swap(i as usize, j as usize);
    }
    perm
}

/// Resolve the track index `AudioOp::Play`/`Next`/`Prev` should land on, given `mode` and the
/// deterministic `shuffle_seed`. `current` is `None` for a fresh `Play`, `Some(index)` for a
/// `Next`/`Prev` advance from an existing entry. A fresh play starts at the mode's NATURAL
/// first track (0 for ordered modes, the seeded order's first element for `Shuffle`) — never
/// one track in.
fn resolve_track_index(
    playlist: &PlaylistEngine,
    mode: PlaylistMode,
    shuffle_seed: u32,
    current: Option<u32>,
    forward: bool,
) -> Option<u32> {
    let len = playlist.tracks.len();
    if len == 0 {
        return None;
    }
    match mode {
        PlaylistMode::Single => Some(current.unwrap_or(0).min(len as u32 - 1)),
        PlaylistMode::Sequential | PlaylistMode::LoopAll => {
            let next = match current {
                // A fresh play starts at the natural first track, never one track in.
                None => 0,
                Some(idx) => {
                    if forward {
                        idx.wrapping_add(1)
                    } else {
                        // (idx + len - 1) % len, not idx.wrapping_sub(1) % len — the wrapping_sub
                        // form turns Prev-from-0 into (2^32 - 1) % len, which only lands on len - 1
                        // when len divides 2^32.
                        (idx + len as u32 - 1) % len as u32
                    }
                }
            };
            if mode == PlaylistMode::LoopAll {
                Some(next % len as u32)
            } else if (next as usize) < len {
                Some(next)
            } else {
                None // Sequential ends: no wraparound.
            }
        }
        PlaylistMode::Shuffle => {
            let perm = seeded_permutation(len as u32, shuffle_seed);
            match current {
                None => Some(perm[0]),
                Some(track_index) => {
                    let pos = perm.iter().position(|&t| t == track_index).unwrap_or(0) as u32;
                    let next = if forward {
                        (pos + 1) % len as u32
                    } else {
                        (pos + len as u32 - 1) % len as u32
                    };
                    Some(perm[next as usize])
                }
            }
        }
    }
}

/// Apply one `AudioOp` to `state`, returning the fresh post-image or a refusal. Pure and
/// I/O-free: `playlist_lookup` resolves a playlist id to its (already-fetched) engine body —
/// the CALLER (`transport::handle_transport`) does the async DB read before calling this, since
/// a `Play`/`Next`/`Prev` never needs more than the one playlist the op itself names or the
/// advancing entry's own `playlist` field.
///
/// # Examples
///
/// ```
/// use shadowcat::audio::state::apply;
/// use shadowcat::data::engine::{AudioChannel, AudioStateEngine};
/// use shadowcat::ws::protocol::AudioOp;
///
/// let state = AudioStateEngine::default();
/// let op = AudioOp::Play {
///     playlist: None,
///     asset: Some("a1".into()),
///     track_index: None,
///     channel: Some(AudioChannel::Sfx),
///     gain: Some(1.0),
///     loop_: Some(false),
/// };
/// let next = apply(&state, &op, 0.0, &|_| None).unwrap();
/// assert_eq!(next.playing.len(), 1);
/// ```
pub fn apply(
    state: &AudioStateEngine,
    op: &AudioOp,
    now: f64,
    playlist_lookup: &dyn Fn(Uuid) -> Option<PlaylistEngine>,
) -> Result<AudioStateEngine, AudioError> {
    let mut next = state.clone();
    match op {
        AudioOp::Play {
            playlist,
            asset,
            track_index,
            channel,
            gain,
            loop_,
        } => {
            if next.playing.len() >= MAX_PLAYING_TRACKS {
                return Err(AudioError::Cap);
            }
            let (resolved_asset, resolved_channel, resolved_gain, resolved_loop, resolved_index) =
                match playlist {
                    Some(pid) => {
                        let pl = playlist_lookup(*pid).ok_or(AudioError::UnknownPlaylist)?;
                        if pl.tracks.is_empty() {
                            return Err(AudioError::EmptyPlaylist);
                        }
                        // A fresh Shuffle play (no explicit track_index) advances the world's
                        // shuffle_seed first, so the order does not repeat identically forever;
                        // the committed seed still drives every client's walk identically.
                        if pl.mode == PlaylistMode::Shuffle && track_index.is_none() {
                            next.shuffle_seed = advance_seed(next.shuffle_seed);
                        }
                        let idx = track_index
                            .or_else(|| {
                                resolve_track_index(&pl, pl.mode, next.shuffle_seed, None, true)
                            })
                            .unwrap_or(0)
                            .min(pl.tracks.len() as u32 - 1);
                        let track = &pl.tracks[idx as usize];
                        (
                            asset.clone().unwrap_or_else(|| track.asset.clone()),
                            channel.unwrap_or(pl.channel),
                            gain.unwrap_or(track.gain),
                            loop_.unwrap_or(track.loop_),
                            idx,
                        )
                    }
                    None => {
                        let a = asset.clone().ok_or(AudioError::UnknownId)?;
                        (
                            a,
                            channel.unwrap_or(AudioChannel::Sfx),
                            gain.unwrap_or(1.0),
                            loop_.unwrap_or(false),
                            track_index.unwrap_or(0),
                        )
                    }
                };
            if !resolved_gain.is_finite() {
                return Err(AudioError::InvalidGain);
            }
            next.playing.push(PlayingTrack {
                id: Uuid::new_v4(),
                playlist: *playlist,
                track_index: resolved_index,
                asset: resolved_asset,
                channel: resolved_channel,
                gain: resolved_gain,
                loop_: resolved_loop,
                started_at: now,
                paused_at: None,
            });
        }
        AudioOp::Pause { id } => {
            let track = next
                .playing
                .iter_mut()
                .find(|t| t.id == *id)
                .ok_or(AudioError::UnknownId)?;
            if track.paused_at.is_none() {
                track.paused_at = Some(now);
            }
        }
        AudioOp::Resume { id } => {
            let track = next
                .playing
                .iter_mut()
                .find(|t| t.id == *id)
                .ok_or(AudioError::UnknownId)?;
            if let Some(paused_at) = track.paused_at.take() {
                // Shift started_at forward by the paused duration, so the resumed position
                // continues exactly where it paused rather than jumping.
                track.started_at += now - paused_at;
            }
        }
        AudioOp::Stop { id } => {
            let before = next.playing.len();
            next.playing.retain(|t| t.id != *id);
            if next.playing.len() == before {
                return Err(AudioError::UnknownId);
            }
        }
        AudioOp::StopAll => {
            next.playing.clear();
        }
        AudioOp::Seek { id, position_ms } => {
            let track = next
                .playing
                .iter_mut()
                .find(|t| t.id == *id)
                .ok_or(AudioError::UnknownId)?;
            track.started_at = now - *position_ms as f64;
            if track.paused_at.is_some() {
                track.paused_at = Some(now);
            }
        }
        AudioOp::Next { id } | AudioOp::Prev { id } => {
            let forward = matches!(op, AudioOp::Next { .. });
            let pos = next
                .playing
                .iter()
                .position(|t| t.id == *id)
                .ok_or(AudioError::UnknownId)?;
            let current = next.playing[pos].clone();
            let Some(pid) = current.playlist else {
                // A direct (playlist-less) entry has nothing to advance to: stop it.
                next.playing.remove(pos);
                return Ok(next);
            };
            let pl = playlist_lookup(pid).ok_or(AudioError::UnknownPlaylist)?;
            match resolve_track_index(
                &pl,
                pl.mode,
                next.shuffle_seed,
                Some(current.track_index),
                forward,
            ) {
                None => {
                    next.playing.remove(pos); // Sequential ran off the end: stop.
                }
                Some(idx) => {
                    let track = &pl.tracks[idx as usize];
                    next.playing[pos] = PlayingTrack {
                        id: Uuid::new_v4(),
                        playlist: Some(pid),
                        track_index: idx,
                        asset: track.asset.clone(),
                        channel: pl.channel,
                        gain: track.gain,
                        loop_: track.loop_,
                        started_at: now,
                        paused_at: None,
                    };
                }
            }
        }
        AudioOp::SetGain { id, gain } => {
            if !gain.is_finite() {
                return Err(AudioError::InvalidGain);
            }
            let track = next
                .playing
                .iter_mut()
                .find(|t| t.id == *id)
                .ok_or(AudioError::UnknownId)?;
            track.gain = *gain;
        }
    }
    Ok(next)
}

#[cfg(test)]
mod tests;
