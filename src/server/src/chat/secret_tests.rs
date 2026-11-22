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

/// The secret segments of a delivered body, outermost first: each entry is the segment's JSON
/// object, descending through `content` where it was delivered.
fn secret_chain(engine: &serde_json::Value) -> Vec<serde_json::Value> {
    let mut chain = Vec::new();
    let mut level = engine["content"].as_array().cloned().unwrap_or_default();
    while let Some(secret) = level.iter().find(|s| s["kind"] == "secret").cloned() {
        level = secret
            .get("content")
            .and_then(|c| c.as_array())
            .cloned()
            .unwrap_or_default();
        chain.push(secret);
    }
    chain
}

/// A whole delivered document, serialized, so a test can assert an id appears nowhere in it.
async fn delivered_text(
    h: &Harness,
    who: &PermissionContext,
    ev: &crate::data::snapshot::StoredCommand,
    id: Uuid,
) -> String {
    let doc = wire_ops(h, who, ev)
        .await
        .into_iter()
        .find_map(|op| match op {
            Operation::Create { doc } if doc.id == id => Some(doc),
            _ => None,
        })
        .expect("the message arrives as a Create");
    serde_json::to_string(&doc).unwrap()
}

// Fails if `push_segment_overrides` stops registering `{prefix}/visibility`: the non-recipient
// copy then keeps `visibility` and its recipient ids.
#[tokio::test]
async fn a_non_recipient_copy_has_neither_visibility_nor_content_at_every_depth() {
    let h = harness().await;
    let friend = h.friend.user_id.to_string();
    let bystander = h.bystander.user_id.to_string();
    let (id, ev) = send_and_event(
        &h,
        &h.sender,
        "[[secret:@friend|outer [[secret:@bystander|inner]] rest]]",
    )
    .await;

    // The author reads both levels and both recipient lists.
    let author = secret_chain(&delivered_engine(&h, &h.sender, &ev, id).await);
    assert_eq!(author.len(), 2, "the author receives both depths");
    for seg in &author {
        assert!(seg.get("visibility").is_some() && seg.get("content").is_some());
    }

    // The friend reads the outer secret, and is excluded from the inner one.
    let friend_copy = delivered_engine(&h, &h.friend, &ev, id).await;
    let chain = secret_chain(&friend_copy);
    assert_eq!(
        chain.len(),
        2,
        "the inner secret is still on the wire as a stub"
    );
    assert!(chain[0].get("visibility").is_some() && chain[0].get("content").is_some());
    assert!(
        chain[1].get("visibility").is_none() && chain[1].get("content").is_none(),
        "depth 2 is withheld from a reader the inner secret excludes: {}",
        chain[1]
    );
    let text = delivered_text(&h, &h.friend, &ev, id).await;
    assert!(
        !text.contains(&bystander),
        "the inner recipient id leaked: {text}"
    );

    // The bystander and the GM are excluded from the outer secret: no key at depth 1.
    for who in [&h.bystander, &h.gm] {
        let chain = secret_chain(&delivered_engine(&h, who, &ev, id).await);
        assert_eq!(chain.len(), 1, "only the stub remains");
        assert!(
            chain[0].get("visibility").is_none() && chain[0].get("content").is_none(),
            "the stub carries neither key: {}",
            chain[0]
        );
        let text = delivered_text(&h, who, &ev, id).await;
        assert!(
            !text.contains(&friend),
            "the outer recipient id leaked: {text}"
        );
    }
}

// Fails if the GM tier stops removing `visibility`: a GM-only secret then reaches a player with
// its `visibility` key.
#[tokio::test]
async fn a_gm_secret_stub_for_a_player_has_no_visibility_and_the_gm_copy_keeps_both() {
    let h = harness().await;
    let (id, ev) = send_and_event(&h, &h.sender, "[[secret:gm|a [[secret:gm|b]]]]").await;

    let gm_chain = secret_chain(&delivered_engine(&h, &h.gm, &ev, id).await);
    assert_eq!(gm_chain.len(), 2);
    for seg in &gm_chain {
        assert_eq!(seg["visibility"], "gm_only");
        assert!(seg.get("content").is_some());
    }
    for who in [&h.sender, &h.friend, &h.bystander] {
        let chain = secret_chain(&delivered_engine(&h, who, &ev, id).await);
        assert_eq!(chain.len(), 1);
        assert!(chain[0].get("visibility").is_none() && chain[0].get("content").is_none());
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

// Fails if `scrub_recipient_lists` stops running on Update changes: the edit's
// `/permissions/property_overrides` write then carries the friend's id to the bystander.
#[tokio::test]
async fn an_edit_that_adds_a_secret_does_not_send_recipient_ids_to_a_non_recipient() {
    let h = harness().await;
    let id = send(&h, &h.sender, "plain", Audience::Public).await;
    let (mut rx, _) = h.room.subscribe();
    let rate = PingRateLimiter::new();
    handle_edit_message(
        MessageRequestCtx {
            room: &h.room,
            repo: &h.repo,
            ctx: &h.sender,
            rate: &rate,
            vfx_rate: &rate,
            preview: LinkPreviewDeps {
                client: &link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 300,
            budget_per_min: 30,
        },
        id,
        "see [[secret:@friend|meet at dawn]]".into(),
    )
    .await
    .unwrap();
    let ev = next_event(&mut rx).await;

    let friend = h.friend.user_id.to_string();
    let bystander_ops = serde_json::to_string(&wire_ops(&h, &h.bystander, &ev).await).unwrap();
    assert!(!bystander_ops.contains(&friend), "leaked: {bystander_ops}");
    assert!(!bystander_ops.contains("dawn"));
    let friend_ops = serde_json::to_string(&wire_ops(&h, &h.friend, &ev).await).unwrap();
    assert!(
        friend_ops.contains(&friend) && friend_ops.contains("dawn"),
        "the named user keeps both: {friend_ops}"
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

// Fails if the `visibility` field loses its serde default: a stub with no key then refuses to
// deserialize and the server cannot index or re-read a redacted copy.
#[test]
fn a_redacted_secret_deserializes_with_empty_content_and_gm_only_visibility() {
    let seg: Segment = serde_json::from_value(serde_json::json!({ "kind": "secret" })).unwrap();
    assert!(matches!(
        seg,
        Segment::Secret { visibility: Visibility::GmOnly, content } if content.is_empty()
    ));
}
