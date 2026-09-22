use super::*;

/// A distinct, valid asset-ref UUID string derived from `i` — every track/entry fixture in
/// this file needs one, since `PlaylistTrack.asset`/`PlayingTrack.asset` are now format-checked
/// by `validate_asset_ref` (a placeholder like `"a0"` no longer passes ingress).
fn test_asset(i: usize) -> String {
    format!("00000000-0000-0000-0000-{i:012}")
}

#[test]
fn playlist_engine_rejects_over_cap_tracks() {
    let tracks = (0..=MAX_PLAYLIST_TRACKS)
        .map(|i| PlaylistTrack {
            asset: test_asset(i),
            name: None,
            gain: 1.0,
            loop_: false,
        })
        .collect();
    let playlist = PlaylistEngine {
        tracks,
        mode: PlaylistMode::Sequential,
        channel: AudioChannel::Music,
        fade_ms: 0,
    };
    assert!(playlist.validate().is_err());
}

#[test]
fn playlist_engine_rejects_empty_asset_id() {
    let playlist = PlaylistEngine {
        tracks: vec![PlaylistTrack {
            asset: String::new(),
            name: None,
            gain: 1.0,
            loop_: false,
        }],
        mode: PlaylistMode::Sequential,
        channel: AudioChannel::Music,
        fade_ms: 0,
    };
    assert!(playlist.validate().is_err());
}

#[test]
fn playlist_engine_rejects_nonfinite_gain() {
    let playlist = PlaylistEngine {
        tracks: vec![PlaylistTrack {
            asset: test_asset(0),
            name: None,
            gain: f64::NAN,
            loop_: false,
        }],
        mode: PlaylistMode::Sequential,
        channel: AudioChannel::Music,
        fade_ms: 0,
    };
    assert!(playlist.validate().is_err());
}

#[test]
fn playlist_engine_rejects_over_cap_fade() {
    let playlist = PlaylistEngine {
        tracks: vec![],
        mode: PlaylistMode::Sequential,
        channel: AudioChannel::Music,
        fade_ms: MAX_PLAYLIST_FADE_MS + 1,
    };
    assert!(playlist.validate().is_err());
}

#[test]
fn audio_state_engine_rejects_over_cap_playing() {
    let playing = (0..=MAX_PLAYING_TRACKS)
        .map(|i| PlayingTrack {
            id: uuid::Uuid::new_v4(),
            playlist: None,
            track_index: 0,
            asset: test_asset(i),
            channel: AudioChannel::Sfx,
            gain: 1.0,
            loop_: false,
            started_at: 0.0,
            paused_at: None,
        })
        .collect();
    let state = AudioStateEngine {
        playing,
        shuffle_seed: 0,
    };
    assert!(state.validate().is_err());
}

#[test]
fn audio_state_engine_default_is_empty_and_valid() {
    let state = AudioStateEngine::default();
    assert!(state.playing.is_empty());
    assert!(state.validate().is_ok());
}
