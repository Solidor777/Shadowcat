use super::*;
use crate::data::engine::{AudioChannel, PlaylistEngine, PlaylistMode, PlaylistTrack};

fn playlist(mode: PlaylistMode, n: usize) -> PlaylistEngine {
    PlaylistEngine {
        tracks: (0..n)
            .map(|i| PlaylistTrack {
                asset: format!("a{i}"),
                name: None,
                gain: 1.0,
                loop_: false,
            })
            .collect(),
        mode,
        channel: AudioChannel::Music,
        fade_ms: 0,
    }
}

#[test]
fn play_direct_asset_with_no_playlist() {
    let state = AudioStateEngine::default();
    let op = AudioOp::Play {
        playlist: None,
        asset: Some("a1".into()),
        track_index: None,
        channel: None,
        gain: None,
        loop_: None,
    };
    let next = apply(&state, &op, 100.0, &|_| None).unwrap();
    assert_eq!(next.playing.len(), 1);
    assert_eq!(next.playing[0].asset, "a1");
    assert_eq!(next.playing[0].channel, AudioChannel::Sfx);
    assert_eq!(next.playing[0].started_at, 100.0);
}

#[test]
fn play_from_playlist_resolves_track_gain_and_channel() {
    let pl = playlist(PlaylistMode::Sequential, 3);
    let pid = uuid::Uuid::new_v4();
    let state = AudioStateEngine::default();
    let op = AudioOp::Play {
        playlist: Some(pid),
        asset: None,
        track_index: Some(1),
        channel: None,
        gain: None,
        loop_: None,
    };
    let next = apply(&state, &op, 0.0, &|id| (id == pid).then(|| pl.clone())).unwrap();
    assert_eq!(next.playing[0].asset, "a1");
    assert_eq!(next.playing[0].channel, AudioChannel::Music);
}

#[test]
fn play_over_cap_is_rejected() {
    let mut state = AudioStateEngine::default();
    for i in 0..MAX_PLAYING_TRACKS {
        state.playing.push(PlayingTrack {
            id: uuid::Uuid::new_v4(),
            playlist: None,
            track_index: 0,
            asset: format!("a{i}"),
            channel: AudioChannel::Sfx,
            gain: 1.0,
            loop_: false,
            started_at: 0.0,
            paused_at: None,
        });
    }
    let op = AudioOp::Play {
        playlist: None,
        asset: Some("over".into()),
        track_index: None,
        channel: None,
        gain: None,
        loop_: None,
    };
    assert_eq!(
        apply(&state, &op, 0.0, &|_| None).unwrap_err(),
        AudioError::Cap
    );
}

#[test]
fn play_unknown_playlist_is_rejected() {
    let state = AudioStateEngine::default();
    let op = AudioOp::Play {
        playlist: Some(uuid::Uuid::new_v4()),
        asset: None,
        track_index: None,
        channel: None,
        gain: None,
        loop_: None,
    };
    assert_eq!(
        apply(&state, &op, 0.0, &|_| None).unwrap_err(),
        AudioError::UnknownPlaylist
    );
}

#[test]
fn play_nonfinite_gain_is_rejected() {
    let state = AudioStateEngine::default();
    let op = AudioOp::Play {
        playlist: None,
        asset: Some("a".into()),
        track_index: None,
        channel: None,
        gain: Some(f64::NAN),
        loop_: None,
    };
    assert_eq!(
        apply(&state, &op, 0.0, &|_| None).unwrap_err(),
        AudioError::InvalidGain
    );
}

#[test]
fn pause_then_resume_shifts_started_at_by_paused_duration() {
    let mut state = AudioStateEngine::default();
    let id = uuid::Uuid::new_v4();
    state.playing.push(PlayingTrack {
        id,
        playlist: None,
        track_index: 0,
        asset: "a".into(),
        channel: AudioChannel::Sfx,
        gain: 1.0,
        loop_: false,
        started_at: 0.0,
        paused_at: None,
    });
    let paused = apply(&state, &AudioOp::Pause { id }, 1_000.0, &|_| None).unwrap();
    assert_eq!(paused.playing[0].paused_at, Some(1_000.0));
    let resumed = apply(&paused, &AudioOp::Resume { id }, 5_000.0, &|_| None).unwrap();
    assert!(resumed.playing[0].paused_at.is_none());
    // Paused for 4000ms; started_at shifts forward by that much so position keeps continuity.
    assert_eq!(resumed.playing[0].started_at, 4_000.0);
}

#[test]
fn stop_unknown_id_is_rejected() {
    let state = AudioStateEngine::default();
    assert_eq!(
        apply(
            &state,
            &AudioOp::Stop {
                id: uuid::Uuid::new_v4()
            },
            0.0,
            &|_| None
        )
        .unwrap_err(),
        AudioError::UnknownId
    );
}

#[test]
fn stop_all_clears_every_entry() {
    let mut state = AudioStateEngine::default();
    state.playing.push(PlayingTrack {
        id: uuid::Uuid::new_v4(),
        playlist: None,
        track_index: 0,
        asset: "a".into(),
        channel: AudioChannel::Sfx,
        gain: 1.0,
        loop_: false,
        started_at: 0.0,
        paused_at: None,
    });
    let next = apply(&state, &AudioOp::StopAll, 0.0, &|_| None).unwrap();
    assert!(next.playing.is_empty());
}

#[test]
fn seek_rewrites_started_at_from_position() {
    let mut state = AudioStateEngine::default();
    let id = uuid::Uuid::new_v4();
    state.playing.push(PlayingTrack {
        id,
        playlist: None,
        track_index: 0,
        asset: "a".into(),
        channel: AudioChannel::Sfx,
        gain: 1.0,
        loop_: false,
        started_at: 0.0,
        paused_at: None,
    });
    let next = apply(
        &state,
        &AudioOp::Seek {
            id,
            position_ms: 30_000,
        },
        100_000.0,
        &|_| None,
    )
    .unwrap();
    assert_eq!(next.playing[0].started_at, 70_000.0);
}

#[test]
fn next_sequential_advances_then_stops_at_end() {
    let pl = playlist(PlaylistMode::Sequential, 2);
    let pid = uuid::Uuid::new_v4();
    let id = uuid::Uuid::new_v4();
    let mut state = AudioStateEngine::default();
    state.playing.push(PlayingTrack {
        id,
        playlist: Some(pid),
        track_index: 0,
        asset: "a0".into(),
        channel: AudioChannel::Music,
        gain: 1.0,
        loop_: false,
        started_at: 0.0,
        paused_at: None,
    });
    let lookup = |q: uuid::Uuid| (q == pid).then(|| pl.clone());
    let advanced = apply(&state, &AudioOp::Next { id }, 1.0, &lookup).unwrap();
    assert_eq!(advanced.playing[0].track_index, 1);
    assert_ne!(advanced.playing[0].id, id); // fresh id on advance
    let ended = apply(
        &advanced,
        &AudioOp::Next {
            id: advanced.playing[0].id,
        },
        2.0,
        &lookup,
    )
    .unwrap();
    assert!(ended.playing.is_empty());
}

#[test]
fn next_loop_all_wraps_to_the_first_track() {
    let pl = playlist(PlaylistMode::LoopAll, 2);
    let pid = uuid::Uuid::new_v4();
    let id = uuid::Uuid::new_v4();
    let mut state = AudioStateEngine::default();
    state.playing.push(PlayingTrack {
        id,
        playlist: Some(pid),
        track_index: 1,
        asset: "a1".into(),
        channel: AudioChannel::Music,
        gain: 1.0,
        loop_: false,
        started_at: 0.0,
        paused_at: None,
    });
    let lookup = |q: uuid::Uuid| (q == pid).then(|| pl.clone());
    let next = apply(&state, &AudioOp::Next { id }, 1.0, &lookup).unwrap();
    assert_eq!(next.playing[0].track_index, 0);
}

#[test]
fn next_direct_entry_with_no_playlist_just_stops() {
    let id = uuid::Uuid::new_v4();
    let mut state = AudioStateEngine::default();
    state.playing.push(PlayingTrack {
        id,
        playlist: None,
        track_index: 0,
        asset: "a".into(),
        channel: AudioChannel::Sfx,
        gain: 1.0,
        loop_: false,
        started_at: 0.0,
        paused_at: None,
    });
    let next = apply(&state, &AudioOp::Next { id }, 1.0, &|_| None).unwrap();
    assert!(next.playing.is_empty());
}

#[test]
fn shuffle_order_is_deterministic_from_the_seed() {
    let pl = playlist(PlaylistMode::Shuffle, 4);
    let pid = uuid::Uuid::new_v4();
    let mut state = AudioStateEngine {
        playing: vec![],
        shuffle_seed: 42,
    };
    let op = AudioOp::Play {
        playlist: Some(pid),
        asset: None,
        track_index: None,
        channel: None,
        gain: None,
        loop_: None,
    };
    let a = apply(&state, &op, 0.0, &|q| (q == pid).then(|| pl.clone())).unwrap();
    state.shuffle_seed = 42;
    let b = apply(&state, &op, 0.0, &|q| (q == pid).then(|| pl.clone())).unwrap();
    // Same seed, same playlist, same call shape ⇒ same resolved starting index every time.
    assert_eq!(a.playing[0].track_index, b.playing[0].track_index);
}

#[test]
fn set_gain_updates_in_place() {
    let mut state = AudioStateEngine::default();
    let id = uuid::Uuid::new_v4();
    state.playing.push(PlayingTrack {
        id,
        playlist: None,
        track_index: 0,
        asset: "a".into(),
        channel: AudioChannel::Sfx,
        gain: 1.0,
        loop_: false,
        started_at: 0.0,
        paused_at: None,
    });
    let next = apply(&state, &AudioOp::SetGain { id, gain: 0.3 }, 0.0, &|_| None).unwrap();
    assert_eq!(next.playing[0].gain, 0.3);
}

#[test]
fn set_gain_nonfinite_is_rejected() {
    let mut state = AudioStateEngine::default();
    let id = uuid::Uuid::new_v4();
    state.playing.push(PlayingTrack {
        id,
        playlist: None,
        track_index: 0,
        asset: "a".into(),
        channel: AudioChannel::Sfx,
        gain: 1.0,
        loop_: false,
        started_at: 0.0,
        paused_at: None,
    });
    assert_eq!(
        apply(
            &state,
            &AudioOp::SetGain {
                id,
                gain: f64::INFINITY
            },
            0.0,
            &|_| None
        )
        .unwrap_err(),
        AudioError::InvalidGain
    );
}

#[test]
fn fresh_play_sequential_starts_at_track_zero() {
    let pl = playlist(PlaylistMode::Sequential, 3);
    let pid = uuid::Uuid::new_v4();
    let state = AudioStateEngine::default();
    let op = AudioOp::Play {
        playlist: Some(pid),
        asset: None,
        track_index: None,
        channel: None,
        gain: None,
        loop_: None,
    };
    let next = apply(&state, &op, 0.0, &|q| (q == pid).then(|| pl.clone())).unwrap();
    assert_eq!(next.playing[0].track_index, 0);
    assert_eq!(next.playing[0].asset, "a0");
}

#[test]
fn loop_all_prev_from_zero_wraps_to_the_last_track() {
    let pl = playlist(PlaylistMode::LoopAll, 3);
    let pid = uuid::Uuid::new_v4();
    let id = uuid::Uuid::new_v4();
    let mut state = AudioStateEngine::default();
    state.playing.push(PlayingTrack {
        id,
        playlist: Some(pid),
        track_index: 0,
        asset: "a0".into(),
        channel: AudioChannel::Music,
        gain: 1.0,
        loop_: false,
        started_at: 0.0,
        paused_at: None,
    });
    let next = apply(&state, &AudioOp::Prev { id }, 1.0, &|q| {
        (q == pid).then(|| pl.clone())
    })
    .unwrap();
    assert_eq!(next.playing[0].track_index, 2);
    assert_eq!(next.playing[0].asset, "a2");
}

#[test]
fn fresh_shuffle_play_starts_at_the_seeded_first_and_advances_the_seed() {
    let pl = playlist(PlaylistMode::Shuffle, 4);
    let pid = uuid::Uuid::new_v4();
    let state = AudioStateEngine {
        playing: vec![],
        shuffle_seed: 7,
    };
    let op = AudioOp::Play {
        playlist: Some(pid),
        asset: None,
        track_index: None,
        channel: None,
        gain: None,
        loop_: None,
    };
    let next = apply(&state, &op, 0.0, &|q| (q == pid).then(|| pl.clone())).unwrap();
    assert_eq!(
        next.playing[0].track_index,
        seeded_permutation(4, advance_seed(7))[0]
    );
    assert_ne!(next.shuffle_seed, state.shuffle_seed);
}

/// Helper: a Play op naming `pid` with every override absent.
fn op_with_playlist(pid: uuid::Uuid) -> AudioOp {
    AudioOp::Play {
        playlist: Some(pid),
        asset: None,
        track_index: None,
        channel: None,
        gain: None,
        loop_: None,
    }
}

#[test]
fn shuffle_next_walk_visits_every_track_exactly_once() {
    for (len, seed) in [(1u32, 0u32), (2, 7), (3, 1), (5, 42), (7, 0x9E37_79B9)] {
        let pl = playlist(PlaylistMode::Shuffle, len as usize);
        let pid = uuid::Uuid::new_v4();
        let lookup = |q: uuid::Uuid| (q == pid).then(|| pl.clone());
        let mut state = AudioStateEngine {
            playing: vec![],
            shuffle_seed: seed,
        };
        // Fresh play lands on the seeded order's first element.
        let first = apply(&state, &op_with_playlist(pid), 0.0, &lookup).unwrap();
        state = first;
        let mut seen: Vec<u32> = vec![state.playing[0].track_index];
        // len - 1 Next advances complete the cycle: every track exactly once.
        for step in 1..len {
            let id = state.playing[0].id;
            state = apply(&state, &AudioOp::Next { id }, step as f64, &lookup).unwrap();
            assert_eq!(
                state.playing.len(),
                1,
                "len {len} seed {seed}: entry survives the walk"
            );
            seen.push(state.playing[0].track_index);
        }
        let mut sorted = seen.clone();
        sorted.sort_unstable();
        assert_eq!(
            sorted,
            (0..len).collect::<Vec<u32>>(),
            "len {len} seed {seed}: full walk covers the playlist"
        );
    }
}

#[test]
fn sequential_fresh_play_does_not_reseed() {
    let pl = playlist(PlaylistMode::Sequential, 3);
    let pid = uuid::Uuid::new_v4();
    let state = AudioStateEngine {
        playing: vec![],
        shuffle_seed: 99,
    };
    let next = apply(&state, &op_with_playlist(pid), 0.0, &|q| {
        (q == pid).then(|| pl.clone())
    })
    .unwrap();
    assert_eq!(next.shuffle_seed, 99);
}
