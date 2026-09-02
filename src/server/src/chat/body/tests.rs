use super::*;
use crate::auth::role::ServerRole;
use crate::chat::rolls::RollError;
use crate::chat::DocLinkTarget;
use crate::data::document::WorldRole;
use crate::data::sqlite::SqliteRepository;

async fn seed_world() -> (SqliteRepository, uuid::Uuid) {
    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("W", gm, 0).await.unwrap();
    (repo, w.id)
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
