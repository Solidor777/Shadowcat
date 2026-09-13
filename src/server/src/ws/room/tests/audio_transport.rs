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
    handle_transport(&repo, &ctx, &room, world, op, 1_000)
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

    let err = handle_transport(&repo, &player_ctx, &room, world, AudioOp::StopAll, 1_000)
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
