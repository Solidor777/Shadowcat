//! A world bundle carries `playlist` documents and the `audio-state` singleton
//! generically: `export_world_rows`/`import_world` operate over the whole
//! `documents` table with no `doc_type` filter, so both ride the same bulk
//! path every other document does — proven here end to end.
use super::*;
use crate::data::document::PermissionSet;
use crate::data::engine::{
    AudioChannel, AudioStateEngine, PlaylistEngine, PlaylistMode, PlaylistTrack,
    AUDIO_STATE_DOC_TYPE, PLAYLIST_DOC_TYPE,
};
use crate::data::world_seed::missing_config_ops;

/// One real playlist engine body (a single track), so the round-trip compares
/// genuine content rather than a default.
fn playlist_engine() -> PlaylistEngine {
    PlaylistEngine {
        tracks: vec![PlaylistTrack {
            asset: "tavern-loop".into(),
            name: Some("Tavern".into()),
            gain: 0.8,
            loop_: true,
        }],
        mode: PlaylistMode::LoopAll,
        channel: AudioChannel::Music,
        fade_ms: 1500,
    }
}

#[tokio::test]
async fn bundle_export_import_carries_playlists_and_the_audio_state_singleton() {
    // Source world, seeded through the ordinary config-seed path.
    let src = repo().await;
    let (world, ctx) = gm_world(&src).await;
    let ops = missing_config_ops(&[], world, None, 0);
    src.apply_intent(&ctx, world, ops, 0, WriteOrigin::ConfigSeed)
        .await
        .unwrap();

    // A playlist through the ordinary client write path (standard write rules).
    let playlist = tests_engine_doc(
        PermissionSet::default(),
        PLAYLIST_DOC_TYPE,
        serde_json::to_value(playlist_engine()).unwrap(),
    );
    let playlist = Document {
        scope: Scope::World { world_id: world },
        ..playlist
    };
    let playlist_id = playlist.id;
    src.apply_intent(
        &ctx,
        world,
        vec![Operation::Create { doc: playlist }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    // Mutate the singleton's seed away from the default, so the import
    // assertion distinguishes "carried from the source world" from
    // "re-seeded fresh at import" (import re-seeds nothing, so the carried
    // value must survive).
    let state_doc = src
        .query_documents(world, AUDIO_STATE_DOC_TYPE)
        .await
        .unwrap()
        .into_iter()
        .next()
        .expect("the seed pass created the audio-state singleton");
    let mutated = AudioStateEngine {
        shuffle_seed: 7,
        ..AudioStateEngine::default()
    };
    src.apply_intent(
        &ctx,
        world,
        vec![Operation::Update {
            doc_id: state_doc.id,
            changes: vec![FieldChange {
                path: "/engine".into(),
                old: state_doc.engine.clone().unwrap(),
                new: serde_json::to_value(&mutated).unwrap(),
                remove: false,
            }],
        }],
        2,
        WriteOrigin::AudioTransport,
    )
    .await
    .unwrap();

    let export = src.export_world_rows(world).await.unwrap();

    // Target: a fresh repository holding the same GM username (member rows
    // re-resolve by name), no world.
    let target = repo().await;
    target
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let import_data = crate::data::world_bundle::WorldImportData {
        manifest: export.manifest,
        documents: export.documents,
        events: export.events,
        members: export.members,
        invites: export.invites,
        assets: export.assets,
        fog: export.fog,
        settings: export.settings,
        staged_assets: vec![],
        staged_siblings: vec![],
    };
    target.import_world(import_data).await.unwrap();

    let imported_playlists = target
        .query_documents(world, PLAYLIST_DOC_TYPE)
        .await
        .unwrap();
    assert_eq!(imported_playlists.len(), 1);
    assert_eq!(imported_playlists[0].id, playlist_id);
    assert_eq!(
        imported_playlists[0].engine.clone().unwrap(),
        serde_json::to_value(playlist_engine()).unwrap()
    );

    let imported_states = target
        .query_documents(world, AUDIO_STATE_DOC_TYPE)
        .await
        .unwrap();
    assert_eq!(imported_states.len(), 1);
    let carried: AudioStateEngine =
        serde_json::from_value(imported_states[0].engine.clone().unwrap()).unwrap();
    assert_eq!(carried.shuffle_seed, 7);
}
