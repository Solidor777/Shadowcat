use super::*;
use crate::auth::role::ServerRole;
use crate::chat::rolls::RollError;
use crate::chat::DocLinkTarget;
use crate::data::asset::{Asset, AssetMeta};
use crate::data::sqlite::SqliteRepository;
use uuid::Uuid;

async fn seed_world() -> (SqliteRepository, Uuid) {
    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("W", gm, 0).await.unwrap();
    (repo, w.id)
}

/// Seeds a minimal in-world asset row and returns its id.
async fn seed_asset(repo: &SqliteRepository, world_id: Uuid) -> Uuid {
    let id = Uuid::new_v4();
    let asset = Asset {
        id,
        world_id,
        storage_key: format!("{world_id}/{id}"),
        original_name: "map.png".to_string(),
        content_type: "image/png".to_string(),
        byte_size: 10,
        created_by: None,
        created_at: 0,
        version: 1,
        folder_id: None,
        tags: Vec::new(),
        derived_tags: Vec::new(),
        meta: AssetMeta::unprocessed("image/png", 10),
    };
    repo.insert_asset(&asset).await.unwrap();
    id
}

#[tokio::test]
async fn text_only_body_matches_sanitize_byte_identically() {
    let (repo, world_id) = seed_world().await;
    let policy = ChatContentPolicy::default();
    let deps = ComposeDeps {
        repo: &repo,
        world_id,
        channel: "general",
        actor_owner: None,
        policy: &policy,
    };
    let got = compose_message("hello world", deps, ScanMode::Execute)
        .await
        .unwrap();
    assert_eq!(got, sanitize::sanitize("hello world", &policy));
}

#[tokio::test]
async fn inline_roll_executes_under_execute_mode() {
    let (repo, world_id) = seed_world().await;
    let policy = ChatContentPolicy::default();
    let deps = ComposeDeps {
        repo: &repo,
        world_id,
        channel: "general",
        actor_owner: None,
        policy: &policy,
    };
    let got = compose_message("roll [[1d6]] now", deps, ScanMode::Execute)
        .await
        .unwrap();
    assert!(
        got.iter().any(|s| matches!(s, Segment::RollEmbed { .. })),
        "expected a RollEmbed segment, got {got:?}"
    );
}

#[tokio::test]
async fn inline_roll_refused_under_no_execute_mode() {
    let (repo, world_id) = seed_world().await;
    let policy = ChatContentPolicy::default();
    let deps = ComposeDeps {
        repo: &repo,
        world_id,
        channel: "general",
        actor_owner: None,
        policy: &policy,
    };
    let err = compose_message("roll [[1d6]] now", deps, ScanMode::NoExecute)
        .await
        .unwrap_err();
    assert!(matches!(err, ComposeError::Inline));
}

#[tokio::test]
async fn button_span_validates_without_rolling() {
    let (repo, world_id) = seed_world().await;
    let policy = ChatContentPolicy::default();
    let deps = ComposeDeps {
        repo: &repo,
        world_id,
        channel: "general",
        actor_owner: None,
        policy: &policy,
    };
    let got = compose_message("[[roll:1d20|Attack]]", deps, ScanMode::NoExecute)
        .await
        .unwrap();
    assert_eq!(
        got,
        vec![Segment::RollButton {
            formula: "1d20".to_string(),
            label: Some("Attack".to_string()),
        }]
    );
}

#[tokio::test]
async fn doc_link_span_stores_target_and_label() {
    let (repo, world_id) = seed_world().await;
    let policy = ChatContentPolicy::default();
    let deps = ComposeDeps {
        repo: &repo,
        world_id,
        channel: "general",
        actor_owner: None,
        policy: &policy,
    };
    let id = "00000000-0000-0000-0000-000000000001";
    let got = compose_message(&format!("[[doc:{id}|My Doc]]"), deps, ScanMode::NoExecute)
        .await
        .unwrap();
    assert_eq!(
        got,
        vec![Segment::DocLink {
            target: DocLinkTarget::Doc {
                doc_id: id.parse().unwrap(),
                embedded_path: None,
            },
            label: "My Doc".to_string(),
        }]
    );
}

#[tokio::test]
async fn a_scan_error_surfaces_as_compose_error_roll() {
    let (repo, world_id) = seed_world().await;
    let policy = ChatContentPolicy::default();
    let deps = ComposeDeps {
        repo: &repo,
        world_id,
        channel: "general",
        actor_owner: None,
        policy: &policy,
    };
    let err = compose_message("[[unterminated", deps, ScanMode::NoExecute)
        .await
        .unwrap_err();
    assert!(matches!(err, ComposeError::Roll(RollError::Unterminated)));
}

fn images_on_policy() -> ChatContentPolicy {
    ChatContentPolicy {
        images: Some(true),
        ..Default::default()
    }
}

#[tokio::test]
async fn asset_span_stores_an_image_segment_when_images_are_enabled() {
    let (repo, world_id) = seed_world().await;
    let asset_id = seed_asset(&repo, world_id).await;
    let policy = images_on_policy();
    let deps = ComposeDeps {
        repo: &repo,
        world_id,
        channel: "general",
        actor_owner: None,
        policy: &policy,
    };
    let got = compose_message(
        &format!("[[asset:{asset_id}|a map]]"),
        deps,
        ScanMode::NoExecute,
    )
    .await
    .unwrap();
    assert_eq!(
        got,
        vec![Segment::Image {
            asset_id,
            alt: "a map".to_string(),
        }]
    );
}

#[tokio::test]
async fn asset_span_with_no_alt_stores_empty_alt() {
    let (repo, world_id) = seed_world().await;
    let asset_id = seed_asset(&repo, world_id).await;
    let policy = images_on_policy();
    let deps = ComposeDeps {
        repo: &repo,
        world_id,
        channel: "general",
        actor_owner: None,
        policy: &policy,
    };
    let got = compose_message(&format!("[[asset:{asset_id}]]"), deps, ScanMode::NoExecute)
        .await
        .unwrap();
    assert_eq!(
        got,
        vec![Segment::Image {
            asset_id,
            alt: String::new(),
        }]
    );
}

#[tokio::test]
async fn asset_span_is_refused_when_images_are_disabled() {
    let (repo, world_id) = seed_world().await;
    let asset_id = seed_asset(&repo, world_id).await;
    let policy = ChatContentPolicy::default();
    let deps = ComposeDeps {
        repo: &repo,
        world_id,
        channel: "general",
        actor_owner: None,
        policy: &policy,
    };
    let err = compose_message(
        &format!("[[asset:{asset_id}|a map]]"),
        deps,
        ScanMode::NoExecute,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ComposeError::Roll(RollError::ImagesDisabled)));
}

#[tokio::test]
async fn asset_span_referencing_a_foreign_world_asset_is_unknown() {
    let (repo, world_id) = seed_world().await;
    let other_gm = repo
        .create_user("other-gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let other_world = repo.create_world_owned("Other", other_gm, 0).await.unwrap();
    let asset_id = seed_asset(&repo, other_world.id).await;
    let policy = images_on_policy();
    let deps = ComposeDeps {
        repo: &repo,
        world_id,
        channel: "general",
        actor_owner: None,
        policy: &policy,
    };
    let err = compose_message(
        &format!("[[asset:{asset_id}|a map]]"),
        deps,
        ScanMode::NoExecute,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ComposeError::Roll(RollError::UnknownAsset)));
}

#[tokio::test]
async fn asset_span_referencing_a_nonexistent_asset_is_unknown() {
    let (repo, world_id) = seed_world().await;
    let policy = images_on_policy();
    let deps = ComposeDeps {
        repo: &repo,
        world_id,
        channel: "general",
        actor_owner: None,
        policy: &policy,
    };
    let err = compose_message(
        &format!("[[asset:{}|a map]]", Uuid::new_v4()),
        deps,
        ScanMode::NoExecute,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ComposeError::Roll(RollError::UnknownAsset)));
}

#[tokio::test]
async fn asset_span_with_a_malformed_id_is_malformed() {
    let (repo, world_id) = seed_world().await;
    let policy = images_on_policy();
    let deps = ComposeDeps {
        repo: &repo,
        world_id,
        channel: "general",
        actor_owner: None,
        policy: &policy,
    };
    let err = compose_message("[[asset:not-a-uuid|a map]]", deps, ScanMode::NoExecute)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        ComposeError::Roll(RollError::MalformedAssetSpan)
    ));
}

#[tokio::test]
async fn asset_span_with_over_long_alt_is_refused() {
    let (repo, world_id) = seed_world().await;
    let asset_id = seed_asset(&repo, world_id).await;
    let policy = images_on_policy();
    let deps = ComposeDeps {
        repo: &repo,
        world_id,
        channel: "general",
        actor_owner: None,
        policy: &policy,
    };
    let long_alt = "a".repeat(crate::chat::MAX_IMAGE_ALT_CHARS + 1);
    let err = compose_message(
        &format!("[[asset:{asset_id}|{long_alt}]]"),
        deps,
        ScanMode::NoExecute,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ComposeError::Roll(RollError::AltTooLong)));
}
