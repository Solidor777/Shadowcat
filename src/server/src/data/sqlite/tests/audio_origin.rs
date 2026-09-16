//! `WriteOrigin::AudioTransport` is the only origin permitted to `Update`
//! `audio-state`; Create/Delete of that singleton are reserved to
//! `WriteOrigin::ConfigSeed` (the world-seed path creates it exactly once).

use super::*;
use crate::data::engine::{AudioStateEngine, AUDIO_STATE_DOC_TYPE};
use crate::data::membership::PermissionContext;

/// A fresh repo + world + GM context; every test in this file writes as the GM.
async fn gm_setup() -> (SqliteRepository, Uuid, PermissionContext) {
    let r = repo().await;
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    (r, w.id, ctx)
}

/// The `audio-state` singleton document, engine body at its default.
fn audio_state_doc(world_id: Uuid) -> Document {
    let mut d = world_doc(700, world_id, serde_json::json!({}));
    d.doc_type = AUDIO_STATE_DOC_TYPE.into();
    d.engine = Some(serde_json::to_value(AudioStateEngine::default()).unwrap());
    d
}

#[tokio::test]
async fn client_origin_cannot_create_audio_state() {
    let (r, world, ctx) = gm_setup().await;
    let doc = audio_state_doc(world);
    let err = r
        .apply_intent(
            &ctx,
            world,
            vec![Operation::Create { doc }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::Forbidden));
}

#[tokio::test]
async fn audio_transport_origin_cannot_create_audio_state() {
    // Create is reserved to ConfigSeed (world_seed's one-time singleton creation) — the
    // transport handler never creates this doc, only updates the one world_seed already made.
    let (r, world, ctx) = gm_setup().await;
    let doc = audio_state_doc(world);
    let err = r
        .apply_intent(
            &ctx,
            world,
            vec![Operation::Create { doc }],
            0,
            WriteOrigin::AudioTransport,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::Forbidden));
}

#[tokio::test]
async fn config_seed_origin_can_create_audio_state() {
    let (r, world, ctx) = gm_setup().await;
    let doc = audio_state_doc(world);
    r.apply_intent(
        &ctx,
        world,
        vec![Operation::Create { doc }],
        0,
        WriteOrigin::ConfigSeed,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn config_seed_origin_cannot_update_audio_state_but_audio_transport_can() {
    let (r, world, ctx) = gm_setup().await;
    let doc = audio_state_doc(world);
    let old_engine = doc.engine.clone().unwrap();
    r.apply_intent(
        &ctx,
        world,
        vec![Operation::Create { doc: doc.clone() }],
        0,
        WriteOrigin::ConfigSeed,
    )
    .await
    .unwrap();
    let next = AudioStateEngine {
        shuffle_seed: 7,
        ..AudioStateEngine::default()
    };
    let change = FieldChange {
        path: "/engine".into(),
        old: old_engine.clone(),
        new: serde_json::to_value(&next).unwrap(),
        remove: false,
    };
    let err = r
        .apply_intent(
            &ctx,
            world,
            vec![Operation::Update {
                doc_id: doc.id,
                changes: vec![change.clone()],
            }],
            1,
            WriteOrigin::ConfigSeed,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DataError::Forbidden));
    r.apply_intent(
        &ctx,
        world,
        vec![Operation::Update {
            doc_id: doc.id,
            changes: vec![change],
        }],
        2,
        WriteOrigin::AudioTransport,
    )
    .await
    .unwrap();
}
