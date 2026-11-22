//! Tests for `Segment::Secret`: the `[[secret:...]]` grammar, the composers' recursion and
//! caps, the property overrides that hide a secret's content, and the per-recipient wire,
//! search and `source` behavior that follows from them.

use super::revise_audience_tests::{harness, next_event, send, wire_ops, Harness};
use super::*;
use crate::data::command::Operation;
use crate::data::membership::PermissionContext;
use crate::data::search::{index_content_gm, index_content_public};
use crate::data::sqlite::SqliteRepository;
use crate::ws::PingRateLimiter;

/// Sends `content` as `who` and returns the stored id plus the event the room broadcast for it.
async fn send_and_event(
    h: &Harness,
    who: &PermissionContext,
    content: &str,
) -> (Uuid, crate::data::snapshot::StoredCommand) {
    let (mut rx, _) = h.room.subscribe();
    let id = send(h, who, content, Audience::Public).await;
    (id, next_event(&mut rx).await)
}

/// The Create delivered to `who` for message `id`, as the JSON the client would receive.
async fn delivered_engine(
    h: &Harness,
    who: &PermissionContext,
    ev: &crate::data::snapshot::StoredCommand,
    id: Uuid,
) -> serde_json::Value {
    wire_ops(h, who, ev)
        .await
        .into_iter()
        .find_map(|op| match op {
            Operation::Create { doc } if doc.id == id => doc.engine,
            _ => None,
        })
        .expect("the message arrives as a Create")
}

/// The `content` array of the first `secret` segment in a delivered engine body, or `None` when
/// the key was stripped.
fn secret_content(engine: &serde_json::Value) -> Option<serde_json::Value> {
    let segs = engine["content"].as_array().expect("content array");
    let secret = segs
        .iter()
        .find(|s| s["kind"] == "secret")
        .expect("a secret segment is on the wire");
    secret.get("content").cloned()
}

/// Composes `body` as the author of a world with one GM and no other members.
async fn compose(body: &str) -> Result<Vec<Segment>, rolls::RollError> {
    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, crate::auth::role::ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("W", gm, 0).await.unwrap();
    let policy = settings::ChatContentPolicy::default();
    let deps = body::ComposeDeps {
        repo: &repo,
        world_id: w.id,
        channel: "general",
        actor_owner: None,
        policy: &policy,
        author: gm,
    };
    match body::compose_message(body, deps, body::ScanMode::Execute).await {
        Ok((segments, _)) => Ok(segments),
        Err(body::ComposeError::Roll(e)) => Err(e),
        Err(other) => panic!("unexpected compose failure {other:?}"),
    }
}

#[tokio::test]
async fn a_gm_secret_reaches_the_gm_and_is_stripped_from_a_player() {
    let h = harness().await;
    let (id, ev) = send_and_event(&h, &h.sender, "open [[secret:gm|vault code 1234]] tail").await;

    let gm_copy = delivered_engine(&h, &h.gm, &ev, id).await;
    let gm_secret = secret_content(&gm_copy).expect("a GM receives the secret's content");
    assert!(gm_secret.to_string().contains("vault code 1234"));

    for who in [&h.bystander, &h.sender] {
        let copy = delivered_engine(&h, who, &ev, id).await;
        assert!(
            secret_content(&copy).is_none(),
            "the key itself is absent for a non-GM, not null or empty"
        );
        assert!(
            copy["content"].to_string().find("1234").is_none(),
            "no trace of the text in the body: {copy}"
        );
    }
}

#[tokio::test]
async fn a_user_secret_reaches_only_the_named_user_and_the_author() {
    let h = harness().await;
    let (id, ev) = send_and_event(&h, &h.sender, "[[secret:@friend|meet at dawn]]").await;

    for who in [&h.friend, &h.sender] {
        let copy = delivered_engine(&h, who, &ev, id).await;
        assert!(
            secret_content(&copy).is_some(),
            "named user and author read it"
        );
    }
    for who in [&h.bystander, &h.gm] {
        let copy = delivered_engine(&h, who, &ev, id).await;
        assert!(
            secret_content(&copy).is_none(),
            "an unlisted player and an unlisted GM do not read it"
        );
        assert!(!copy.to_string().contains("dawn"));
    }
}

#[tokio::test]
async fn nested_secrets_are_stripped_at_every_depth() {
    let h = harness().await;
    let (id, ev) = send_and_event(
        &h,
        &h.sender,
        "[[secret:gm|outer [[secret:@friend|inner]] rest]]",
    )
    .await;

    let gm_copy = delivered_engine(&h, &h.gm, &ev, id).await;
    let outer = secret_content(&gm_copy).expect("the GM reads the outer secret");
    let outer_text = outer.to_string();
    assert!(outer_text.contains("outer"));
    assert!(
        !outer_text.contains("inner"),
        "the GM is not listed on the inner secret, so its content is stripped: {outer_text}"
    );

    let friend_copy = delivered_engine(&h, &h.friend, &ev, id).await;
    assert!(
        !friend_copy.to_string().contains("inner"),
        "the outer GM-only strip removes the inner secret from the friend too"
    );
}

#[tokio::test]
async fn source_is_kept_for_the_author_only_when_a_secret_is_present() {
    let h = harness().await;
    let (id, ev) = send_and_event(&h, &h.sender, "hi [[secret:gm|hidden]]").await;

    let author_copy = delivered_engine(&h, &h.sender, &ev, id).await;
    assert!(author_copy["source"].as_str().unwrap().contains("hidden"));
    for who in [&h.gm, &h.bystander, &h.friend] {
        let copy = delivered_engine(&h, who, &ev, id).await;
        assert!(
            copy.get("source").is_none(),
            "source would leak the secret text"
        );
    }

    let (plain_id, plain_ev) = send_and_event(&h, &h.sender, "no secrets here").await;
    let plain = delivered_engine(&h, &h.bystander, &plain_ev, plain_id).await;
    assert_eq!(
        plain["source"], "no secrets here",
        "a message with no secret keeps `source` readable"
    );
}

#[tokio::test]
async fn secret_text_is_indexed_only_for_a_reader_the_tier_admits() {
    let h = harness().await;
    let id = send(
        &h,
        &h.sender,
        "pub [[secret:gm|gmword]] [[secret:@friend|friendword]]",
        Audience::Public,
    )
    .await;
    let stored = h.repo.get_document(id).await.unwrap().unwrap();

    let public = index_content_public(&stored);
    assert!(public.contains("pub"));
    assert!(!public.contains("gmword") && !public.contains("friendword"));

    let gm = index_content_gm(&stored);
    assert!(gm.contains("gmword"), "the GM partition keeps GM-only text");
    assert!(
        !gm.contains("friendword"),
        "recipient-only text is indexed in no shared partition"
    );
}

#[tokio::test]
async fn an_edit_recomputes_the_overrides_for_the_new_content() {
    let h = harness().await;
    let id = send(&h, &h.sender, "plain", Audience::Public).await;
    assert!(h
        .repo
        .get_document(id)
        .await
        .unwrap()
        .unwrap()
        .permissions
        .property_overrides
        .is_empty());

    let rate = PingRateLimiter::new();
    let edit = |content: &str| {
        let content = content.to_string();
        let h = &h;
        let rate = &rate;
        async move {
            handle_edit_message(
                MessageRequestCtx {
                    room: &h.room,
                    repo: &h.repo,
                    ctx: &h.sender,
                    rate,
                    vfx_rate: rate,
                    preview: LinkPreviewDeps {
                        client: &link_preview::build_client_allow_loopback(),
                        cache: &LinkPreviewCache::new(),
                        rate: &PreviewRateLimiter::new(),
                    },
                    now: 300,
                    budget_per_min: 30,
                },
                id,
                content,
            )
            .await
            .unwrap();
        }
    };

    edit("now [[secret:gm|added]]").await;
    let stored = h.repo.get_document(id).await.unwrap().unwrap();
    let overrides = &stored.permissions.property_overrides;
    assert_eq!(overrides["/engine/content/1/content"], Visibility::GmOnly);
    assert!(overrides.contains_key("/engine/source"));

    edit("secret removed").await;
    let stored = h.repo.get_document(id).await.unwrap().unwrap();
    assert!(
        stored.permissions.property_overrides.is_empty(),
        "dropping the secret drops its overrides"
    );
}

#[tokio::test]
async fn the_grammar_refuses_malformed_unknown_and_oversized_secrets() {
    assert!(matches!(
        compose("[[secret:gm no pipe]]").await,
        Err(rolls::RollError::MalformedSecret)
    ));
    assert!(matches!(
        compose("[[secret:everyone|x]]").await,
        Err(rolls::RollError::MalformedSecret)
    ));
    assert!(matches!(
        compose("[[secret:gm|  ]]").await,
        Err(rolls::RollError::MalformedSecret)
    ));
    assert!(matches!(
        compose("[[secret:@nobody|x]]").await,
        Err(rolls::RollError::UnknownSecretRecipient)
    ));
    let many = (0..=MAX_WHISPER_RECIPIENTS)
        .map(|i| format!("@u{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    assert!(matches!(
        compose(&format!("[[secret:{many}|x]]")).await,
        Err(rolls::RollError::TooManySecretRecipients)
    ));
}

#[tokio::test]
async fn secret_depth_is_capped_and_spans_share_one_budget() {
    let at_cap = format!(
        "{}x{}",
        "[[secret:gm|".repeat(rolls::MAX_SECRET_DEPTH),
        "]]".repeat(rolls::MAX_SECRET_DEPTH)
    );
    assert!(
        compose(&at_cap).await.is_ok(),
        "MAX_SECRET_DEPTH levels compose"
    );

    let over_cap = format!(
        "{}x{}",
        "[[secret:gm|".repeat(rolls::MAX_SECRET_DEPTH + 1),
        "]]".repeat(rolls::MAX_SECRET_DEPTH + 1)
    );
    // Each nested secret is one span, so the 9th level overruns the 8-span budget before the
    // depth check can fire; the depth cap itself is exercised through `compose_static`.
    assert!(matches!(
        compose(&over_cap).await,
        Err(rolls::RollError::TooManyInline(_))
    ));

    // 1 outer span + 8 spans inside it is 9, past MAX_INLINE_ROLLS = 8, though each scan alone
    // stays within the cap.
    let inner = "[[1d6]] ".repeat(rolls::MAX_INLINE_ROLLS);
    assert!(matches!(
        compose(&format!("[[secret:gm|{inner}]]")).await,
        Err(rolls::RollError::TooManyInline(_))
    ));
}

#[test]
fn compose_static_builds_gm_secrets_and_refuses_user_secrets() {
    let policy = settings::NOTE_CONTENT_POLICY;
    let segs = compose_static("a [[secret:gm|b]] c", &policy, 64).unwrap();
    assert!(matches!(
        segs.as_slice(),
        [
            _,
            Segment::Secret {
                visibility: Visibility::GmOnly,
                ..
            },
            _
        ]
    ));
    assert!(matches!(
        compose_static("[[secret:@friend|b]]", &policy, 64),
        Err(rolls::RollError::SecretRequiresGm)
    ));
    let too_deep = format!(
        "{}x{}",
        "[[secret:gm|".repeat(rolls::MAX_SECRET_DEPTH + 1),
        "]]".repeat(rolls::MAX_SECRET_DEPTH + 1)
    );
    assert!(matches!(
        compose_static(&too_deep, &policy, 64),
        Err(rolls::RollError::SecretTooDeep)
    ));
}

#[test]
fn a_redacted_secret_deserializes_with_empty_content() {
    let seg: Segment = serde_json::from_value(serde_json::json!({
        "kind": "secret",
        "visibility": "gm_only",
    }))
    .unwrap();
    assert!(matches!(seg, Segment::Secret { content, .. } if content.is_empty()));
}
