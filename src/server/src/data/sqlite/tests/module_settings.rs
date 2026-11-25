//! The write gate on the seeded `module-settings` singleton: a member below GM cannot write a
//! module's `/system/<moduleId>` blob.

use super::*;
use crate::auth::role::ServerRole;
use crate::data::engine::MODULE_SETTINGS_DOC_TYPE;
use crate::data::membership::PermissionContext;

/// The `module-settings` document as the world seed pass creates it, committed under
/// `WriteOrigin::ConfigSeed`.
async fn seeded_module_settings(
    r: &SqliteRepository,
    gm: &PermissionContext,
    world: Uuid,
) -> Document {
    let doc = crate::data::world_seed::missing_config_ops(&[], world, None, 0)
        .into_iter()
        .find_map(|op| match op {
            Operation::Create { doc } if doc.doc_type == MODULE_SETTINGS_DOC_TYPE => Some(doc),
            _ => None,
        })
        .expect("the seed pass creates the module-settings singleton");
    r.apply_intent(
        gm,
        world,
        vec![Operation::Create { doc: doc.clone() }],
        0,
        WriteOrigin::ConfigSeed,
    )
    .await
    .unwrap();
    doc
}

/// A write of `{"volume": 3}` into the `some-module` blob.
fn blob_write(doc_id: Uuid) -> Operation {
    Operation::Update {
        doc_id,
        changes: vec![FieldChange {
            remove: false,
            path: "/system/some-module".into(),
            old: serde_json::Value::Null,
            new: serde_json::json!({ "volume": 3 }),
        }],
    }
}

#[tokio::test]
async fn a_player_cannot_write_a_module_settings_blob_but_the_gm_can() {
    let r = repo().await;
    let (w, gm) = gm_world(&r).await;
    let player = r.create_user("p", None, ServerRole::User, 0).await.unwrap();
    r.add_member(w, player, WorldRole::Player).await.unwrap();
    let doc = seeded_module_settings(&r, &gm, w).await;

    let as_player = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let refused = r
        .apply_intent(
            &as_player,
            w,
            vec![blob_write(doc.id)],
            1,
            WriteOrigin::Client,
        )
        .await;
    assert!(
        matches!(refused, Err(DataError::Forbidden)),
        "a player's module-settings write must be refused, got {refused:?}"
    );

    r.apply_intent(&gm, w, vec![blob_write(doc.id)], 1, WriteOrigin::Client)
        .await
        .expect("the GM's write is the positive control");
}
