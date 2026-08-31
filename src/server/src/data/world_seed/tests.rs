use super::*;
use crate::auth::role::ServerRole;
use crate::data::validation;

fn world() -> Uuid {
    Uuid::from_u128(0x77)
}

/// Extract the Create docs an empty world's seed pass produces.
fn seed_docs(now: i64) -> Vec<Document> {
    missing_config_ops(&[], world(), None, now)
        .into_iter()
        .map(|op| match op {
            Operation::Create { doc } => doc,
            other => panic!("empty world must yield only Creates, got {other:?}"),
        })
        .collect()
}

#[test]
fn empty_world_yields_ten_creates_with_seed_bodies() {
    let docs = seed_docs(5);
    assert_eq!(docs.len(), 10);
    for doc in &docs {
        assert_eq!(doc.scope, Scope::World { world_id: world() });
        assert_eq!(doc.owner, None);
        assert_eq!(doc.parent_id, None);
        assert_eq!(doc.name, None);
        // Mirrors the shipped client envelope: readable by every member.
        assert_eq!(doc.permissions.default, DocRole::Observer);
        assert!(doc.permissions.users.is_empty());
        assert_eq!((doc.created_at, doc.updated_at), (5, 5));
        let mut d = doc.clone();
        validation::validate_engine_tree(&mut d).expect("every seed body validates");
    }
    let mut types: Vec<&str> = docs.iter().map(|d| d.doc_type.as_str()).collect();
    let mut expect: Vec<&str> = CONFIG_SINGLETON_DOC_TYPES.to_vec();
    types.sort_unstable();
    expect.sort_unstable();
    assert_eq!(types, expect);

    let body = |ty: &str| {
        docs.iter()
            .find(|d| d.doc_type == ty)
            .and_then(|d| d.engine.clone())
            .expect("seeded engine body present")
    };
    assert_eq!(
        body(FACTION_REGISTRY_DOC_TYPE),
        serde_json::to_value(FactionRegistryEngine::seed()).unwrap()
    );
    assert_eq!(
        body(CONDITION_REGISTRY_DOC_TYPE),
        serde_json::to_value(ConditionRegistryEngine::seed()).unwrap()
    );
    assert_eq!(
        body(CHANNEL_REGISTRY_DOC_TYPE),
        serde_json::to_value(ChannelRegistryEngine::seed()).unwrap()
    );
    assert_eq!(
        body(VISION_MODES_DOC_TYPE),
        serde_json::to_value(VisionModesEngine::seed()).unwrap()
    );
    assert_eq!(
        body(LIGHT_GRADATION_DOC_TYPE),
        serde_json::to_value(LightGradationEngine::seed()).unwrap()
    );
    assert_eq!(
        body(crate::chat::CHAT_SETTINGS_DOC_TYPE),
        serde_json::to_value(ChatSettingsEngine::default()).unwrap()
    );
    assert_eq!(
        body(crate::chat::DICE_SETTINGS_DOC_TYPE),
        serde_json::to_value(DiceSettingsEngine::default()).unwrap()
    );
    assert_eq!(
        body(RESOURCE_REGISTRY_DOC_TYPE),
        serde_json::to_value(ResourceRegistryEngine::default()).unwrap()
    );
    assert_eq!(
        body(WORLD_SETTINGS_DOC_TYPE),
        serde_json::to_value(WorldSettingsEngine::default()).unwrap()
    );
    assert_eq!(
        body(SYSTEM_DEFAULTS_DOC_TYPE),
        serde_json::to_value(SystemDefaultsEngine::default()).unwrap()
    );
}

#[test]
fn a_full_set_yields_no_ops() {
    let docs = seed_docs(0);
    assert!(missing_config_ops(&docs, world(), None, 1).is_empty());
}

#[test]
fn only_the_absent_singleton_is_created() {
    let docs: Vec<Document> = seed_docs(0)
        .into_iter()
        .filter(|d| d.doc_type != FACTION_REGISTRY_DOC_TYPE)
        .collect();
    let ops = missing_config_ops(&docs, world(), None, 1);
    assert_eq!(ops.len(), 1);
    match &ops[0] {
        Operation::Create { doc } => assert_eq!(doc.doc_type, FACTION_REGISTRY_DOC_TYPE),
        other => panic!("expected a Create, got {other:?}"),
    }
}

#[test]
fn system_defaults_drift_yields_an_occ_update() {
    let docs = seed_docs(0);
    let declared = SystemDefaultsEngine {
        scene: Some(crate::data::engine::SceneDefaultsOverlay {
            fog: Some(false),
            ..Default::default()
        }),
        ..Default::default()
    };
    let ops = missing_config_ops(&docs, world(), Some(&declared), 1);
    assert_eq!(ops.len(), 1);
    let stored_doc = docs
        .iter()
        .find(|d| d.doc_type == SYSTEM_DEFAULTS_DOC_TYPE)
        .unwrap();
    match &ops[0] {
        Operation::Update { doc_id, changes } => {
            assert_eq!(*doc_id, stored_doc.id);
            assert_eq!(changes.len(), 1);
            let ch = &changes[0];
            assert!(!ch.remove);
            assert_eq!(ch.path, "/engine");
            assert_eq!(ch.old, stored_doc.engine.clone().unwrap());
            assert_eq!(ch.new, serde_json::to_value(&declared).unwrap());
        }
        other => panic!("expected an Update, got {other:?}"),
    }
}

#[test]
fn a_declared_system_seeds_the_create_body() {
    let docs: Vec<Document> = seed_docs(0)
        .into_iter()
        .filter(|d| d.doc_type != SYSTEM_DEFAULTS_DOC_TYPE)
        .collect();
    let declared = SystemDefaultsEngine {
        pathfinding: Some(crate::data::engine::PathfindingOverlay {
            diagonal_rule: Some(crate::data::engine::DiagonalRule::Euclidean),
        }),
        ..Default::default()
    };
    let ops = missing_config_ops(&docs, world(), Some(&declared), 1);
    assert_eq!(ops.len(), 1);
    match &ops[0] {
        Operation::Create { doc } => {
            assert_eq!(doc.doc_type, SYSTEM_DEFAULTS_DOC_TYPE);
            assert_eq!(
                doc.engine.clone().unwrap(),
                serde_json::to_value(&declared).unwrap()
            );
        }
        other => panic!("expected a Create, got {other:?}"),
    }
}

#[tokio::test]
async fn enabled_system_defaults_resolves_the_single_enabled_system_provider() {
    let r = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    // Nothing installed/enabled: no system layer.
    assert!(enabled_system_defaults(&r, w.id, dir.path())
        .await
        .is_none());
    std::fs::create_dir_all(dir.path().join("sys")).unwrap();
    std::fs::write(
        dir.path().join("sys").join("module.json"),
        r#"{"id":"sys","version":"1.0.0","engines":{"shadowcat":"*"},"provides":[{"contract":"shadowcat.system","cardinality":"singleton"}],"systemDefaults":{"scene":{"fog":false}}}"#,
    )
    .unwrap();
    // Installed but not enabled: still no system layer.
    assert!(enabled_system_defaults(&r, w.id, dir.path())
        .await
        .is_none());
    r.set_world_enabled_modules(w.id, &["sys".to_string()])
        .await
        .unwrap();
    let sd = enabled_system_defaults(&r, w.id, dir.path())
        .await
        .expect("enabled system's declaration resolves");
    assert_eq!(sd.scene.unwrap().fog, Some(false));
}

#[tokio::test]
async fn seed_author_is_the_first_gm_by_user_id_and_none_without_a_gm() {
    let r = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm_a = r
        .create_user("gm-a", None, ServerRole::User, 0)
        .await
        .unwrap();
    let gm_b = r
        .create_user("gm-b", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = r.create_user("p", None, ServerRole::User, 0).await.unwrap();
    let w = r.create_world_owned("W", gm_a, 0).await.unwrap();
    r.add_member(w.id, gm_b, WorldRole::Gm).await.unwrap();
    r.add_member(w.id, player, WorldRole::Player).await.unwrap();
    let ctx = seed_author(&r, w.id).await.expect("a GM exists");
    assert_eq!(ctx.user_id, gm_a.min(gm_b));
    assert_eq!(ctx.world_role, WorldRole::Gm);
    // A world with no GM member (legacy fixture shape): no author, no seed.
    let w2 = r.create_world("W2", 0).await.unwrap();
    assert!(seed_author(&r, w2.id).await.is_none());
}
