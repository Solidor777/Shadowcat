use super::*;
use crate::audio::transport::{handle_transport, TransportError};
use crate::data::command::{FieldChange, Operation, WriteOrigin};
use crate::data::engine::{
    AudioChannel, AudioStateEngine, Grid, PlaylistEngine, PlaylistMode, PlaylistTrack, SceneEngine,
    AUDIO_STATE_DOC_TYPE, PLAYLIST_DOC_TYPE, WORLD_SETTINGS_DOC_TYPE,
};
use crate::data::world_seed::missing_config_ops;
use crate::ws::protocol::AudioOp;

/// Seeds the world's config singletons (including `audio-state`) through the
/// ordinary config-seed path, then opens the world's room.
async fn seeded_room(
    repo: &SqliteRepository,
    world: Uuid,
    ctx: &PermissionContext,
) -> std::sync::Arc<Room> {
    let ops = missing_config_ops(&[], world, None, 0);
    repo.apply_intent(ctx, world, ops, 0, WriteOrigin::ConfigSeed)
        .await
        .unwrap();
    let reg = RoomRegistry::new();
    reg.get_or_create(repo, world).await.unwrap().unwrap()
}

/// A one-track playlist document (LoopAll, Music channel).
fn playlist_doc(world: Uuid) -> crate::data::document::Document {
    let engine = PlaylistEngine {
        tracks: vec![PlaylistTrack {
            asset: "tavern-loop".into(),
            name: None,
            gain: 0.8,
            loop_: false,
        }],
        mode: PlaylistMode::LoopAll,
        channel: AudioChannel::Music,
        fade_ms: 0,
    };
    crate::data::document::Document {
        id: Uuid::new_v4(),
        scope: crate::data::document::Scope::World { world_id: world },
        doc_type: PLAYLIST_DOC_TYPE.into(),
        schema_version: 1,
        name: Some("P".into()),
        source: None,
        base: None,
        owner: None,
        permissions: Default::default(),
        embedded: Default::default(),
        parent_id: None,
        engine: Some(serde_json::to_value(engine).unwrap()),
        system: serde_json::json!({}),
        created_at: 0,
        updated_at: 0,
    }
}

/// A scene document whose `SceneEngine.ambience` names `playlist` at `gain`.
fn scene_doc(world: Uuid, playlist: Uuid, gain: f64) -> crate::data::document::Document {
    let engine = SceneEngine {
        grid: Grid {
            kind: "square".into(),
            size: 100.0,
            distance: None,
        },
        background: None,
        bounds: None,
        snap_to_grid: None,
        vision: None,
        lighting: None,
        combat: None,
        ambience: Some(crate::data::engine::scene::SceneAmbience { playlist, gain }),
    };
    crate::data::document::Document {
        id: Uuid::new_v4(),
        scope: crate::data::document::Scope::World { world_id: world },
        doc_type: "scene".into(),
        schema_version: 1,
        name: Some("S".into()),
        source: None,
        base: None,
        owner: None,
        permissions: Default::default(),
        embedded: Default::default(),
        parent_id: None,
        engine: Some(serde_json::to_value(engine).unwrap()),
        system: serde_json::json!({}),
        created_at: 0,
        updated_at: 0,
    }
}

/// The world's current `AudioStateEngine` (post-image of every committed
/// transport write).
async fn audio_state(repo: &SqliteRepository, world: Uuid) -> AudioStateEngine {
    let doc = repo
        .query_documents(world, AUDIO_STATE_DOC_TYPE)
        .await
        .unwrap()
        .into_iter()
        .next()
        .expect("the seed pass created the audio-state singleton");
    crate::data::engine::engine_of(&doc)
}

#[tokio::test]
async fn a_gm_play_through_handle_transport_commits_the_new_playing_entry() {
    let (repo, world, ctx) = repo_with_world().await;
    let room = seeded_room(&repo, world, &ctx).await;
    let playlist = playlist_doc(world);
    let playlist_id = playlist.id;
    repo.apply_intent(
        &ctx,
        world,
        vec![Operation::Create { doc: playlist }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let op = AudioOp::Play {
        playlist: Some(playlist_id),
        asset: None,
        track_index: None,
        channel: None,
        gain: None,
        loop_: None,
    };
    handle_transport(&repo, &ctx, &room, op, 1_000)
        .await
        .unwrap();

    let state = audio_state(&repo, world).await;
    assert_eq!(state.playing.len(), 1);
    let entry = &state.playing[0];
    assert_eq!(entry.playlist, Some(playlist_id));
    assert_eq!(entry.asset, "tavern-loop");
    assert_eq!(entry.channel, AudioChannel::Music);
    assert_eq!(entry.started_at, 1_000.0);
}

#[tokio::test]
async fn a_player_transport_op_is_forbidden_and_commits_nothing() {
    let (repo, world, ctx) = repo_with_world().await;
    let room = seeded_room(&repo, world, &ctx).await;
    let player = repo
        .create_user("p", None, ServerRole::User, 0)
        .await
        .unwrap();
    repo.add_member(world, player, WorldRole::Player)
        .await
        .unwrap();
    let player_ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };

    let err = handle_transport(&repo, &player_ctx, &room, AudioOp::StopAll, 1_000)
        .await
        .unwrap_err();
    assert!(matches!(err, TransportError::Forbidden));
    assert!(audio_state(&repo, world).await.playing.is_empty());
}

#[tokio::test]
async fn activating_a_scene_starts_its_ambience_and_leaving_it_stops() {
    let (repo, world, ctx) = repo_with_world().await;
    let room = seeded_room(&repo, world, &ctx).await;
    let playlist = playlist_doc(world);
    let playlist_id = playlist.id;
    let scene = scene_doc(world, playlist_id, 0.6);
    let scene_id = scene.id;
    repo.apply_intent(
        &ctx,
        world,
        vec![
            Operation::Create { doc: playlist },
            Operation::Create { doc: scene },
        ],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let ws_doc = repo
        .query_documents(world, WORLD_SETTINGS_DOC_TYPE)
        .await
        .unwrap()
        .into_iter()
        .next()
        .expect("the seed pass created world-settings");

    // Activate the scene: the ambience playlist starts playing server-side.
    let activate = FieldChange {
        path: "/engine/activeScene".into(),
        old: serde_json::Value::Null,
        new: serde_json::Value::String(scene_id.to_string()),
        remove: false,
    };
    room.publish(
        &repo,
        &ctx,
        vec![Operation::Update {
            doc_id: ws_doc.id,
            changes: vec![activate],
        }],
        2,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    let state = audio_state(&repo, world).await;
    assert_eq!(state.playing.len(), 1);
    let entry = &state.playing[0];
    assert_eq!(entry.playlist, Some(playlist_id));
    assert_eq!(entry.channel, AudioChannel::Ambience);
    assert_eq!(entry.gain, 0.6);

    // Deactivate: the ambience entry stops (matched by playlist id).
    let deactivate = FieldChange {
        path: "/engine/activeScene".into(),
        old: serde_json::Value::String(scene_id.to_string()),
        new: serde_json::Value::Null,
        remove: false,
    };
    room.publish(
        &repo,
        &ctx,
        vec![Operation::Update {
            doc_id: ws_doc.id,
            changes: vec![deactivate],
        }],
        3,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    assert!(audio_state(&repo, world).await.playing.is_empty());
}

/// An audio asset row with a known `duration_ms` (the TrackEnded gate's duration source).
async fn insert_audio_asset(repo: &SqliteRepository, world: Uuid, id: Uuid, duration_ms: i64) {
    repo.insert_asset(&crate::data::asset::Asset {
        id,
        world_id: world,
        storage_key: format!("{world}/{id}"),
        original_name: "loop.wav".into(),
        content_type: "audio/wav".into(),
        byte_size: 100,
        created_by: None,
        created_at: 0,
        version: 1,
        folder_id: None,
        tags: vec![],
        derived_tags: vec![],
        meta: crate::data::asset::AssetMeta {
            duration_ms: Some(duration_ms),
            ..crate::data::asset::AssetMeta::unprocessed("audio/wav", 100)
        },
    })
    .await
    .unwrap();
}

/// A one-track LoopAll playlist whose track names `asset` directly.
fn playlist_doc_with_asset(world: Uuid, asset: Uuid) -> crate::data::document::Document {
    let mut doc = playlist_doc(world);
    doc.engine = Some(
        serde_json::to_value(PlaylistEngine {
            tracks: vec![PlaylistTrack {
                asset: asset.to_string(),
                name: None,
                gain: 1.0,
                loop_: false,
            }],
            mode: PlaylistMode::LoopAll,
            channel: AudioChannel::Music,
            fade_ms: 0,
        })
        .unwrap(),
    );
    doc
}

/// Play `playlist_id` through the handler as the GM at `now`.
async fn gm_play(
    repo: &SqliteRepository,
    ctx: &PermissionContext,
    room: &std::sync::Arc<Room>,
    playlist_id: Uuid,
    now: i64,
) {
    let op = AudioOp::Play {
        playlist: Some(playlist_id),
        asset: None,
        track_index: None,
        channel: None,
        gain: None,
        loop_: None,
    };
    handle_transport(repo, ctx, room, op, now).await.unwrap();
}

#[tokio::test]
async fn track_ended_reports_obey_the_pause_aware_elapsed_gate() {
    let (repo, world, ctx) = repo_with_world().await;
    let room = seeded_room(&repo, world, &ctx).await;
    let asset = Uuid::new_v4();
    insert_audio_asset(&repo, world, asset, 5_000).await;
    let playlist = playlist_doc_with_asset(world, asset);
    let playlist_id = playlist.id;
    repo.apply_intent(
        &ctx,
        world,
        vec![Operation::Create { doc: playlist }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    gm_play(&repo, &ctx, &room, playlist_id, 1_000).await;
    let first_id = audio_state(&repo, world).await.playing[0].id;

    // A PLAYER's premature report is a silent no-op (elapsed 2000 < 5000).
    let player = repo
        .create_user("p", None, ServerRole::User, 0)
        .await
        .unwrap();
    repo.add_member(world, player, WorldRole::Player)
        .await
        .unwrap();
    let player_ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    handle_transport(
        &repo,
        &player_ctx,
        &room,
        AudioOp::TrackEnded { id: first_id },
        3_000,
    )
    .await
    .unwrap();
    assert_eq!(audio_state(&repo, world).await.playing[0].id, first_id);

    // Pause at 2000: the paused span must not count as playback time. A report at 7000
    // wall-clock still sees only 1000ms elapsed.
    handle_transport(&repo, &ctx, &room, AudioOp::Pause { id: first_id }, 2_000)
        .await
        .unwrap();
    handle_transport(
        &repo,
        &player_ctx,
        &room,
        AudioOp::TrackEnded { id: first_id },
        7_000,
    )
    .await
    .unwrap();
    assert_eq!(audio_state(&repo, world).await.playing[0].id, first_id);

    // Resume at 8000 (started_at shifts to 7000); at 12500 the elapsed 5500ms passes the gate.
    handle_transport(&repo, &ctx, &room, AudioOp::Resume { id: first_id }, 8_000)
        .await
        .unwrap();
    handle_transport(
        &repo,
        &player_ctx,
        &room,
        AudioOp::TrackEnded { id: first_id },
        12_500,
    )
    .await
    .unwrap();
    let advanced = audio_state(&repo, world).await;
    assert_eq!(advanced.playing.len(), 1);
    assert_ne!(
        advanced.playing[0].id, first_id,
        "the advance assigns a fresh id"
    );

    // A second report naming the now-stale id is a silent no-op (the first report won).
    let second_id = advanced.playing[0].id;
    handle_transport(
        &repo,
        &player_ctx,
        &room,
        AudioOp::TrackEnded { id: first_id },
        13_000,
    )
    .await
    .unwrap();
    assert_eq!(audio_state(&repo, world).await.playing[0].id, second_id);
}

/// An audio asset row whose duration was never probed (`meta.duration_ms == None`) — the
/// reachable state after a transcode failure or an over-cap upload.
async fn insert_audio_asset_no_duration(repo: &SqliteRepository, world: Uuid, id: Uuid) {
    repo.insert_asset(&crate::data::asset::Asset {
        id,
        world_id: world,
        storage_key: format!("{world}/{id}"),
        original_name: "loop.wav".into(),
        content_type: "audio/wav".into(),
        byte_size: 100,
        created_by: None,
        created_at: 0,
        version: 1,
        folder_id: None,
        tags: vec![],
        derived_tags: vec![],
        meta: crate::data::asset::AssetMeta::unprocessed("audio/wav", 100),
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn track_ended_with_an_unresolvable_duration_never_advances() {
    let (repo, world, ctx) = repo_with_world().await;
    let room = seeded_room(&repo, world, &ctx).await;
    let asset = Uuid::new_v4();
    insert_audio_asset_no_duration(&repo, world, asset).await;
    let playlist = playlist_doc_with_asset(world, asset);
    let playlist_id = playlist.id;
    repo.apply_intent(
        &ctx,
        world,
        vec![Operation::Create { doc: playlist }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    gm_play(&repo, &ctx, &room, playlist_id, 1_000).await;
    let first_id = audio_state(&repo, world).await.playing[0].id;

    let player = repo
        .create_user("p", None, ServerRole::User, 0)
        .await
        .unwrap();
    repo.add_member(world, player, WorldRole::Player)
        .await
        .unwrap();
    let player_ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    // A report arriving well after any plausible track length still refuses: with no
    // resolvable duration, the report can never be confirmed non-premature.
    handle_transport(
        &repo,
        &player_ctx,
        &room,
        AudioOp::TrackEnded { id: first_id },
        1_000_000,
    )
    .await
    .unwrap();
    assert_eq!(
        audio_state(&repo, world).await.playing[0].id,
        first_id,
        "an unresolvable duration must never let TrackEnded advance the track"
    );
}

#[tokio::test]
async fn a_gm_next_skips_unconditionally_even_before_the_duration_elapses() {
    let (repo, world, ctx) = repo_with_world().await;
    let room = seeded_room(&repo, world, &ctx).await;
    let asset = Uuid::new_v4();
    insert_audio_asset(&repo, world, asset, 60_000).await;
    let playlist = playlist_doc_with_asset(world, asset);
    let playlist_id = playlist.id;
    repo.apply_intent(
        &ctx,
        world,
        vec![Operation::Create { doc: playlist }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();
    gm_play(&repo, &ctx, &room, playlist_id, 1_000).await;
    let first_id = audio_state(&repo, world).await.playing[0].id;

    // 500ms into a 60s track: the GM's explicit skip advances immediately, no gate.
    handle_transport(&repo, &ctx, &room, AudioOp::Next { id: first_id }, 1_500)
        .await
        .unwrap();
    assert_ne!(audio_state(&repo, world).await.playing[0].id, first_id);
}

#[tokio::test]
async fn a_playlist_from_another_world_is_refused_as_unknown() {
    let (repo, world, ctx) = repo_with_world().await;
    let room = seeded_room(&repo, world, &ctx).await;
    // A playlist belonging to a DIFFERENT world.
    let other = repo.create_world_owned("W2", ctx.user_id, 0).await.unwrap();
    let foreign = playlist_doc(other.id);
    let foreign_id = foreign.id;
    repo.apply_intent(
        &ctx,
        other.id,
        vec![Operation::Create { doc: foreign }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let err = handle_transport(
        &repo,
        &ctx,
        &room,
        AudioOp::Play {
            playlist: Some(foreign_id),
            asset: None,
            track_index: None,
            channel: None,
            gain: None,
            loop_: None,
        },
        1_000,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        err,
        TransportError::Op(crate::audio::state::AudioError::UnknownPlaylist)
    ));
    assert!(audio_state(&repo, world).await.playing.is_empty());
}
