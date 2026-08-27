use super::*;
use crate::data::command::Operation;
use crate::data::document::{DocRole, Scope};
use uuid::Uuid;

#[test]
fn roll_embed_carries_roll_id_spec_raw_and_defaults_recalc_history_to_none() {
    let spec = crate::dice::notation::parse(
        "1d6",
        crate::dice::ParseContext {
            mode: crate::dice::notation::ModeKind::Total,
            direction: crate::dice::spec::Direction::HighWins,
        },
    )
    .unwrap();
    let mut rng = crate::dice::rng::NoiseRng::from_seed(1);
    let raw = crate::dice::roll(&spec, &mut rng);
    let outcome = crate::dice::evaluate(&spec, &raw);
    let seg = Segment::RollEmbed {
        formula: "1d6".into(),
        outcome,
        roll_id: Uuid::from_u128(1),
        spec: Some(Box::new(spec.clone())),
        raw: Some(Box::new(raw.clone())),
        recalc_history: None,
    };
    let j = serde_json::to_value(&seg).unwrap();
    assert_eq!(j["roll_id"], serde_json::json!(Uuid::from_u128(1)));
    assert!(j.get("spec").is_some(), "spec must serialize when Some");
    assert!(j.get("raw").is_some(), "raw must serialize when Some");
    assert!(
        j.get("recalc_history").is_none(),
        "recalc_history: None must not serialize (skip_serializing_if)"
    );
    let back: Segment = serde_json::from_value(j).unwrap();
    assert_eq!(back, seg);
}

#[test]
fn command_message_id_extracts_create_and_update_doc_ids() {
    let world_id = Uuid::new_v4();
    let doc = seed_actor_doc(Uuid::new_v4(), world_id, None);
    let create = Command {
        seq: 1,
        world_id,
        author: Uuid::new_v4(),
        ts: 0,
        ops: vec![Operation::Create { doc: doc.clone() }],
    };
    assert_eq!(command_message_id(&create), Some(doc.id));

    let update_target = Uuid::new_v4();
    let update = Command {
        seq: 2,
        world_id,
        author: Uuid::new_v4(),
        ts: 0,
        ops: vec![Operation::Update {
            doc_id: update_target,
            changes: vec![],
        }],
    };
    assert_eq!(command_message_id(&update), Some(update_target));

    let delete = Command {
        seq: 3,
        world_id,
        author: Uuid::new_v4(),
        ts: 0,
        ops: vec![Operation::Delete { doc: doc.clone() }],
    };
    assert_eq!(
        command_message_id(&delete),
        None,
        "a Delete carries no id the post-publish pipeline would republish against"
    );
}

#[test]
fn roll_embed_without_roll_id_deserializes_with_a_fresh_generated_one() {
    // A roll embedded before this field existed has no `roll_id` key at all —
    // `#[serde(default = "Uuid::new_v4")]` fills one in rather than failing to parse.
    let old_json = serde_json::json!({
        "kind": "roll_embed",
        "formula": "1d6",
        "outcome": {
            "total": 3, "records": [], "successes": null, "pass": null, "margin": null,
            "tier_label": null, "tier_value": null, "crit_successes": 0, "crit_fails": 0,
            "positive_counter": 0, "negative_counter": 0, "symbol_counts": {}, "labeled_consts": []
        }
    });
    let seg: Segment = serde_json::from_value(old_json).unwrap();
    match seg {
        Segment::RollEmbed {
            roll_id,
            spec,
            raw,
            recalc_history,
            ..
        } => {
            assert_ne!(roll_id, Uuid::nil());
            assert!(spec.is_none(), "a pre-existing roll has no stored spec");
            assert!(raw.is_none(), "a pre-existing roll has no stored raw");
            assert!(recalc_history.is_none());
        }
        other => panic!("expected RollEmbed, got {other:?}"),
    }
}

#[test]
fn roll_embed_property_overrides_marks_spec_raw_and_recalc_history_previous_raw_gm_only() {
    use crate::data::document::Visibility;

    let spec = crate::dice::notation::parse(
        "1d6",
        crate::dice::ParseContext {
            mode: crate::dice::notation::ModeKind::Total,
            direction: crate::dice::spec::Direction::HighWins,
        },
    )
    .unwrap();
    let mut rng = crate::dice::rng::NoiseRng::from_seed(3);
    let raw = crate::dice::roll(&spec, &mut rng);
    let outcome = crate::dice::evaluate(&spec, &raw);

    let content = vec![
        Segment::Text {
            text: "before ".into(),
        },
        Segment::RollEmbed {
            formula: "1d6".into(),
            outcome: outcome.clone(),
            roll_id: Uuid::from_u128(1),
            spec: Some(Box::new(spec.clone())),
            raw: Some(Box::new(raw.clone())),
            recalc_history: Some(vec![RecalcEntry {
                ops: vec![crate::dice::RecalcOp::RerollDice(vec![0])],
                previous_raw: raw.clone(),
                previous_outcome: outcome,
                recalculated_by: Uuid::from_u128(2),
                recalculated_at: 100,
            }]),
        },
    ];

    let overrides = roll_embed_property_overrides(&content);
    assert_eq!(
        overrides.get("/engine/content/1/spec"),
        Some(&Visibility::GmOnly)
    );
    assert_eq!(
        overrides.get("/engine/content/1/raw"),
        Some(&Visibility::GmOnly)
    );
    assert_eq!(
        overrides.get("/engine/content/1/recalc_history/0/previous_raw"),
        Some(&Visibility::GmOnly)
    );
    assert_eq!(
        overrides.get("/engine/content/1/recalc_history/0/previous_outcome"),
        None,
        "previous_outcome is visible to every recipient, never gm_only"
    );
    assert_eq!(overrides.len(), 3, "no other pointers should be marked");
}

#[test]
fn roll_embed_property_overrides_is_empty_for_non_roll_content() {
    let content = vec![Segment::Text { text: "hi".into() }];
    assert!(roll_embed_property_overrides(&content).is_empty());
}

#[test]
fn roll_embed_property_overrides_skips_a_pre_existing_roll_with_no_spec_raw() {
    // A roll embedded before this feature shipped: spec/raw are None, so no
    // override entries should be produced for it (nothing to hide).
    let outcome = crate::dice::evaluate(
        &crate::dice::notation::parse(
            "1d6",
            crate::dice::ParseContext {
                mode: crate::dice::notation::ModeKind::Total,
                direction: crate::dice::spec::Direction::HighWins,
            },
        )
        .unwrap(),
        &crate::dice::roll(
            &crate::dice::notation::parse(
                "1d6",
                crate::dice::ParseContext {
                    mode: crate::dice::notation::ModeKind::Total,
                    direction: crate::dice::spec::Direction::HighWins,
                },
            )
            .unwrap(),
            &mut crate::dice::rng::NoiseRng::from_seed(4),
        ),
    );
    let content = vec![Segment::RollEmbed {
        formula: "1d6".into(),
        outcome,
        roll_id: Uuid::from_u128(5),
        spec: None,
        raw: None,
        recalc_history: None,
    }];
    assert!(roll_embed_property_overrides(&content).is_empty());
}

#[test]
fn recalc_entry_round_trips() {
    let spec = crate::dice::notation::parse(
        "1d6",
        crate::dice::ParseContext {
            mode: crate::dice::notation::ModeKind::Total,
            direction: crate::dice::spec::Direction::HighWins,
        },
    )
    .unwrap();
    let mut rng = crate::dice::rng::NoiseRng::from_seed(2);
    let raw = crate::dice::roll(&spec, &mut rng);
    let outcome = crate::dice::evaluate(&spec, &raw);
    let entry = RecalcEntry {
        ops: vec![crate::dice::RecalcOp::RerollDice(vec![0])],
        previous_raw: raw,
        previous_outcome: outcome,
        recalculated_by: Uuid::from_u128(9),
        recalculated_at: 1000,
    };
    let j = serde_json::to_value(&entry).unwrap();
    let back: RecalcEntry = serde_json::from_value(j).unwrap();
    assert_eq!(back, entry);
}

#[test]
fn html_segment_tagged_roundtrip() {
    let s = Segment::Html {
        sanitized_html: "<em>hi</em>".into(),
    };
    let j = serde_json::to_value(&s).unwrap();
    assert_eq!(j["kind"], "html");
    assert_eq!(j["sanitized_html"], "<em>hi</em>");
    assert_eq!(s, serde_json::from_value(j).unwrap());
}

#[test]
fn message_system_omits_absent_edit_delete_markers() {
    let sys = MessageEngine {
        channel: "all".into(),
        user_owner: Uuid::from_u128(1),
        actor_owner: None,
        kind: MessageKind::Normal,
        audience: Audience::Public,
        content: plain_text_content("hi"),
        source: None,
        edited_at: None,
        deleted_at: None,
    };
    let j = serde_json::to_value(&sys).unwrap();
    assert!(
        j.get("edited_at").is_none(),
        "None edited_at must not serialize"
    );
    assert!(
        j.get("deleted_at").is_none(),
        "None deleted_at must not serialize"
    );
    // Round-trips (a stored message with no markers deserializes unchanged).
    assert_eq!(sys, serde_json::from_value(j).unwrap());
}

#[test]
fn build_message_doc_threads_kind() {
    let doc = build_message_doc(
        Uuid::from_u128(10),
        Uuid::from_u128(20),
        MessageDraft {
            channel: "all".into(),
            actor_owner: None,
            audience: Audience::Public,
            kind: MessageKind::Emote,
            content: plain_text_content("waves"),
            source: None,
        },
        1,
    );
    let sys: MessageEngine = serde_json::from_value(doc.engine.unwrap()).unwrap();
    assert_eq!(sys.kind, MessageKind::Emote);
}

#[test]
fn actor_owner_ref_tagged_roundtrip() {
    let a = ActorOwnerRef::Actor {
        actor_id: Uuid::from_u128(1),
    };
    let j = serde_json::to_value(&a).unwrap();
    assert_eq!(j["kind"], "actor");
    assert_eq!(a, serde_json::from_value(j).unwrap());

    let t = ActorOwnerRef::TokenInstance {
        token_id: Uuid::from_u128(2),
    };
    let j = serde_json::to_value(&t).unwrap();
    assert_eq!(j["kind"], "token_instance");
    assert_eq!(t, serde_json::from_value(j).unwrap());
}

#[test]
fn message_kind_defaults_normal_snake_case() {
    assert_eq!(MessageKind::default(), MessageKind::Normal);
    assert_eq!(
        serde_json::to_value(MessageKind::System).unwrap(),
        serde_json::json!("system")
    );
}

#[test]
fn plain_text_produces_single_text_segment() {
    let segs = plain_text_content("hello <b>world</b>");
    assert_eq!(
        segs,
        vec![Segment::Text {
            text: "hello <b>world</b>".into()
        }]
    );
    // Producer stores raw text verbatim; markup is inert data, rendered as text.
    let j = serde_json::to_value(&segs[0]).unwrap();
    assert_eq!(j["kind"], "text");
    assert_eq!(j["text"], "hello <b>world</b>");
}

#[test]
fn plain_text_empty_is_empty_segment() {
    assert_eq!(
        plain_text_content(""),
        vec![Segment::Text {
            text: String::new()
        }]
    );
}

#[test]
fn build_message_doc_is_server_owned_message() {
    let world = Uuid::from_u128(10);
    let user = Uuid::from_u128(20);
    let doc = build_message_doc(
        world,
        user,
        MessageDraft {
            channel: "all".into(),
            actor_owner: None,
            audience: Audience::Public,
            kind: MessageKind::Normal,
            content: plain_text_content("hi"),
            source: None,
        },
        1234,
    );
    assert_eq!(doc.doc_type, MESSAGE_DOC_TYPE);
    assert_eq!(doc.owner, Some(user));
    assert_eq!(doc.scope, Scope::World { world_id: world });
    assert_eq!(doc.created_at, 1234);
    // Author gets the Owner floor so the create WRITE_FIELDS check passes;
    // default Observer so every world member can read it.
    assert_eq!(doc.permissions.default, DocRole::Observer);
    assert_eq!(doc.permissions.users.get(&user), Some(&DocRole::Owner));
    // Body round-trips back to a MessageEngine with server-set user_owner.
    let sys: MessageEngine = serde_json::from_value(doc.engine.clone().unwrap()).unwrap();
    assert_eq!(sys.user_owner, user);
    assert_eq!(sys.channel, "all");
    assert_eq!(sys.kind, MessageKind::Normal);
    assert_eq!(sys.audience, Audience::Public);
    assert_eq!(sys.content, vec![Segment::Text { text: "hi".into() }]);
}

#[test]
fn ops_target_message_detects_message_create_and_update() {
    let msg = build_message_doc(
        Uuid::from_u128(1),
        Uuid::from_u128(2),
        MessageDraft {
            channel: "all".into(),
            actor_owner: None,
            audience: Audience::Public,
            kind: MessageKind::Normal,
            content: vec![],
            source: None,
        },
        0,
    );
    assert!(ops_target_message(&[Operation::Create {
        doc: msg.clone()
    }]));
    assert!(ops_target_message(&[Operation::Delete { doc: msg }]));

    let mut note = build_message_doc(
        Uuid::from_u128(1),
        Uuid::from_u128(2),
        MessageDraft {
            channel: "all".into(),
            actor_owner: None,
            audience: Audience::Public,
            kind: MessageKind::Normal,
            content: vec![],
            source: None,
        },
        0,
    );
    note.doc_type = "note".into();
    assert!(!ops_target_message(&[Operation::Create { doc: note }]));
}

#[test]
fn ops_target_message_detects_message_in_mixed_batch() {
    // A batch with one innocuous non-message op followed by a message
    // Create must still trip the guard: `.any()` must not short-circuit
    // on the first (non-matching) op.
    let mut note = build_message_doc(
        Uuid::from_u128(1),
        Uuid::from_u128(2),
        MessageDraft {
            channel: "all".into(),
            actor_owner: None,
            audience: Audience::Public,
            kind: MessageKind::Normal,
            content: vec![],
            source: None,
        },
        0,
    );
    note.doc_type = "note".into();
    let msg = build_message_doc(
        Uuid::from_u128(1),
        Uuid::from_u128(2),
        MessageDraft {
            channel: "all".into(),
            actor_owner: None,
            audience: Audience::Public,
            kind: MessageKind::Normal,
            content: vec![],
            source: None,
        },
        0,
    );
    assert!(ops_target_message(&[
        Operation::Create { doc: note },
        Operation::Create { doc: msg },
    ]));
}

#[test]
fn audience_tagged_roundtrip_and_default() {
    let w = Audience::Whisper {
        recipients: vec![Uuid::from_u128(1)],
    };
    let j = serde_json::to_value(&w).unwrap();
    assert_eq!(j["kind"], "whisper");
    assert_eq!(w, serde_json::from_value(j).unwrap());
    assert_eq!(
        serde_json::to_value(Audience::GmOnly).unwrap()["kind"],
        "gm_only"
    );
    assert_eq!(Audience::default(), Audience::Public);
}

#[test]
fn build_message_doc_public_matches_c1_shape() {
    let owner = Uuid::from_u128(1);
    let doc = build_message_doc(
        Uuid::from_u128(9),
        owner,
        MessageDraft {
            channel: "all".into(),
            actor_owner: None,
            audience: Audience::Public,
            kind: MessageKind::Normal,
            content: plain_text_content("hi"),
            source: None,
        },
        0,
    );
    assert_eq!(doc.permissions.default, DocRole::Observer);
    assert_eq!(doc.permissions.gm_role, None);
    assert_eq!(doc.permissions.users.get(&owner), Some(&DocRole::Owner));
}

#[test]
fn build_message_doc_whisper_restricts_default_and_gm() {
    let owner = Uuid::from_u128(1);
    let recipient = Uuid::from_u128(2);
    let doc = build_message_doc(
        Uuid::from_u128(9),
        owner,
        MessageDraft {
            channel: "whispers".into(),
            actor_owner: None,
            audience: Audience::Whisper {
                recipients: vec![recipient],
            },
            kind: MessageKind::Normal,
            content: plain_text_content("psst"),
            source: None,
        },
        0,
    );
    assert_eq!(doc.permissions.default, DocRole::None);
    assert_eq!(doc.permissions.gm_role, Some(DocRole::None));
    assert_eq!(doc.permissions.users.get(&owner), Some(&DocRole::Owner));
    assert_eq!(
        doc.permissions.users.get(&recipient),
        Some(&DocRole::Observer)
    );
}

#[test]
fn build_message_doc_whisper_self_recipient_does_not_downgrade_owner() {
    let owner = Uuid::from_u128(1);
    let doc = build_message_doc(
        Uuid::from_u128(9),
        owner,
        MessageDraft {
            channel: "whispers".into(),
            actor_owner: None,
            audience: Audience::Whisper {
                recipients: vec![owner],
            },
            kind: MessageKind::Normal,
            content: plain_text_content("note to self"),
            source: None,
        },
        0,
    );
    assert_eq!(
        doc.permissions.users.get(&owner),
        Some(&DocRole::Owner),
        "a redundant self-recipient must never downgrade the owner to Observer"
    );
}

#[test]
fn build_message_doc_gm_only_has_no_named_recipients() {
    let owner = Uuid::from_u128(1);
    let doc = build_message_doc(
        Uuid::from_u128(9),
        owner,
        MessageDraft {
            channel: "gm".into(),
            actor_owner: None,
            audience: Audience::GmOnly,
            kind: MessageKind::Normal,
            content: plain_text_content("only the GM sees this"),
            source: None,
        },
        0,
    );
    assert_eq!(doc.permissions.default, DocRole::None);
    assert_eq!(doc.permissions.gm_role, Some(DocRole::Observer));
    assert_eq!(
        doc.permissions.users.len(),
        1,
        "only the owner is individually listed — every GM sees it dynamically via gm_role"
    );
    assert_eq!(doc.permissions.users.get(&owner), Some(&DocRole::Owner));
}

#[test]
fn send_message_error_display_classifies_authorization_vs_validation() {
    use super::SendMessageError as E;

    // Validation-class (safe to surface verbatim): each describes the SENDER'S
    // OWN input or an immutable product rule, disclosing nothing about another
    // user's permissions, a document's existence, or ownership structure.
    assert_eq!(E::Empty.to_string(), "Message cannot be empty.");
    assert_eq!(E::TooLong.to_string(), "Message is too long.");
    assert_eq!(
        E::RateLimited.to_string(),
        "You are sending messages too quickly. Please wait a moment."
    );
    assert_eq!(
        E::UnknownRecipient.to_string(),
        "One or more whisper recipients are not members of this world."
    );
    assert_eq!(
        E::AudienceLocked.to_string(),
        "You cannot change who can see a message after it is sent."
    );
    assert_eq!(
        E::RollImmutable.to_string(),
        "A roll cannot be edited once it has been sent."
    );

    // Authorization-class (GENERIC only): surfacing the specific reason would
    // let a sender probe permission/ownership/existence structure.
    let generic_send = "You are not permitted to send this message.";
    let generic_modify = "You are not permitted to modify this message.";
    // The [sec] variant: never disclose whether the actor exists or who owns it.
    assert_eq!(E::ActorNotSpeakable.to_string(), generic_send);
    assert_eq!(E::Forbidden.to_string(), generic_modify);
    // Existence-oracle close: NotFound is INDISTINGUISHABLE from Forbidden, so a
    // caller cannot tell "message doesn't exist" from "message isn't yours".
    assert_eq!(E::NotFound.to_string(), generic_modify);
    assert_eq!(E::NotFound.to_string(), E::Forbidden.to_string());

    // Internal error: generic; must NEVER leak the inner DataError detail (which
    // can carry SQL / constraint / path text).
    let secret = "unique_constraint_secret_column";
    let data = E::Data(DataError::Conflict(secret.to_string())).to_string();
    assert_eq!(
        data,
        "The message could not be delivered. Please try again."
    );
    assert!(
        !data.contains(secret),
        "Data(_) Display must not leak the inner DataError detail"
    );

    // Roll: never reaches the wire error channel (caught upstream and authored
    // as a System notice), but Display must be total and player-safe.
    assert!(!E::Roll(rolls::RollError::Unterminated)
        .to_string()
        .is_empty());
}

#[tokio::test]
async fn handle_send_message_publishes_and_broadcasts() {
    use crate::auth::role::ServerRole;
    use crate::data::document::WorldRole;
    use crate::data::sqlite::SqliteRepository;
    use crate::ws::room::RoomRegistry;

    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = repo
        .create_user("pl", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("W", gm, 0).await.unwrap();
    repo.add_member(w.id, player, WorldRole::Player)
        .await
        .unwrap();
    let ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };

    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
    let (mut rx, _current) = room.subscribe();
    let rate = PingRateLimiter::new();

    let (cmd, _pending) = handle_send_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &super::link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 30,
        },
        "all".into(),
        "hello".into(),
        None,
        Audience::Public,
    )
    .await
    .unwrap();
    assert_eq!(cmd.seq, 1);
    let got = rx.recv().await.unwrap();
    assert_eq!(got.event_seq(), Some(1));

    // Rate limit: exhaust the budget then expect RateLimited.
    let rate2 = PingRateLimiter::new();
    for _ in 0..2 {
        let _ = handle_send_message(
            MessageRequestCtx {
                room: &room,
                repo: &repo,
                ctx: &ctx,
                rate: &rate2,
                preview: LinkPreviewDeps {
                    client: &super::link_preview::build_client_allow_loopback(),
                    cache: &LinkPreviewCache::new(),
                    rate: &PreviewRateLimiter::new(),
                },
                now: 100,
                budget_per_min: 2,
            },
            "all".into(),
            "x".into(),
            None,
            Audience::Public,
        )
        .await;
    }
    let err = handle_send_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate2,
            preview: LinkPreviewDeps {
                client: &super::link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 2,
        },
        "all".into(),
        "x".into(),
        None,
        Audience::Public,
    )
    .await;
    assert!(matches!(err, Err(SendMessageError::RateLimited)));

    // Empty + too-long rejected before any publish.
    assert!(matches!(
        handle_send_message(
            MessageRequestCtx {
                room: &room,
                repo: &repo,
                ctx: &ctx,
                rate: &rate,
                preview: LinkPreviewDeps {
                    client: &super::link_preview::build_client_allow_loopback(),
                    cache: &LinkPreviewCache::new(),
                    rate: &PreviewRateLimiter::new()
                },
                now: 100,
                budget_per_min: 30,
            },
            "all".into(),
            "".into(),
            None,
            Audience::Public,
        )
        .await,
        Err(SendMessageError::Empty)
    ));
    let long = "a".repeat(MAX_MESSAGE_CHARS + 1);
    assert!(matches!(
        handle_send_message(
            MessageRequestCtx {
                room: &room,
                repo: &repo,
                ctx: &ctx,
                rate: &rate,
                preview: LinkPreviewDeps {
                    client: &super::link_preview::build_client_allow_loopback(),
                    cache: &LinkPreviewCache::new(),
                    rate: &PreviewRateLimiter::new()
                },
                now: 100,
                budget_per_min: 30,
            },
            "all".into(),
            long,
            None,
            Audience::Public,
        )
        .await,
        Err(SendMessageError::TooLong)
    ));

    // Empty/over-long channel rejected before any publish; seq unchanged.
    let seq_before = room.subscribe().1;
    assert!(matches!(
        handle_send_message(
            MessageRequestCtx {
                room: &room,
                repo: &repo,
                ctx: &ctx,
                rate: &rate,
                preview: LinkPreviewDeps {
                    client: &super::link_preview::build_client_allow_loopback(),
                    cache: &LinkPreviewCache::new(),
                    rate: &PreviewRateLimiter::new()
                },
                now: 100,
                budget_per_min: 30,
            },
            "".into(),
            "hi".into(),
            None,
            Audience::Public,
        )
        .await,
        Err(SendMessageError::Empty)
    ));
    let long_channel = "c".repeat(MAX_CHANNEL_CHARS + 1);
    assert!(matches!(
        handle_send_message(
            MessageRequestCtx {
                room: &room,
                repo: &repo,
                ctx: &ctx,
                rate: &rate,
                preview: LinkPreviewDeps {
                    client: &super::link_preview::build_client_allow_loopback(),
                    cache: &LinkPreviewCache::new(),
                    rate: &PreviewRateLimiter::new()
                },
                now: 100,
                budget_per_min: 30,
            },
            long_channel,
            "hi".into(),
            None,
            Audience::Public,
        )
        .await,
        Err(SendMessageError::TooLong)
    ));
    assert_eq!(
        room.subscribe().1,
        seq_before,
        "rejected channel must not publish"
    );
}

#[tokio::test]
async fn a_roll_messages_spec_and_raw_are_gm_only_but_outcome_and_roll_id_are_not() {
    use crate::data::document::WorldRole as DocWorldRole;
    use crate::data::permission::{filter_properties, resolve_access};
    use crate::data::sqlite::SqliteRepository;
    use crate::ws::room::RoomRegistry;

    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, crate::auth::role::ServerRole::User, 0)
        .await
        .unwrap();
    let player = repo
        .create_user("pl", None, crate::auth::role::ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("W", gm, 0).await.unwrap();
    repo.add_member(w.id, player, DocWorldRole::Player)
        .await
        .unwrap();
    let ctx = PermissionContext {
        user_id: player,
        world_role: DocWorldRole::Player,
    };
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
    let rate = PingRateLimiter::new();

    let (cmd, _pending) = handle_send_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 30,
        },
        "all".into(),
        "/roll 1d6".into(),
        None,
        Audience::Public,
    )
    .await
    .unwrap();
    let doc = match &cmd.ops[0] {
        Operation::Create { doc } => doc.clone(),
        other => panic!("expected Create, got {other:?}"),
    };

    // Non-GM player: spec/raw are nulled; outcome/roll_id survive.
    let player_access = resolve_access(player, DocWorldRole::Player, &doc, Some(player));
    let player_view = filter_properties(&doc, &player_access).unwrap();
    let sys: serde_json::Value = player_view.engine.clone().unwrap();
    let seg = &sys["content"][0];
    assert_eq!(seg["spec"], serde_json::Value::Null);
    assert_eq!(seg["raw"], serde_json::Value::Null);
    assert!(seg.get("outcome").is_some() && !seg["outcome"].is_null());
    assert!(seg.get("roll_id").is_some() && !seg["roll_id"].is_null());

    // GM: spec/raw survive unredacted.
    let gm_access = resolve_access(gm, DocWorldRole::Gm, &doc, Some(player));
    let gm_view = filter_properties(&doc, &gm_access).unwrap();
    let gm_sys: serde_json::Value = gm_view.engine.clone().unwrap();
    let gm_seg = &gm_sys["content"][0];
    assert!(gm_seg.get("spec").is_some() && !gm_seg["spec"].is_null());
    assert!(gm_seg.get("raw").is_some() && !gm_seg["raw"].is_null());
}

#[tokio::test]
async fn send_message_stores_a_doc_link_segment() {
    use crate::auth::role::ServerRole;
    use crate::data::document::WorldRole;
    use crate::data::sqlite::SqliteRepository;
    use crate::ws::room::RoomRegistry;

    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = repo
        .create_user("pl", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("W", gm, 0).await.unwrap();
    repo.add_member(w.id, player, WorldRole::Player)
        .await
        .unwrap();

    let ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
    let rate = PingRateLimiter::new();
    let target_id = Uuid::new_v4();

    let (cmd, _pending) = handle_send_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 0,
            budget_per_min: 30,
        },
        "all".into(),
        format!("see [[doc:{target_id}|My Doc]] please"),
        None,
        Audience::Public,
    )
    .await
    .unwrap();
    let doc = match &cmd.ops[0] {
        Operation::Create { doc } => doc,
        other => panic!("expected Create, got {other:?}"),
    };
    let sys: MessageEngine = serde_json::from_value(doc.engine.clone().unwrap()).unwrap();
    assert_eq!(
        sys.content,
        vec![
            Segment::Text {
                text: "see ".into()
            },
            Segment::DocLink {
                target: DocLinkTarget::Doc {
                    doc_id: target_id,
                    embedded_path: None,
                },
                label: "My Doc".into(),
            },
            Segment::Text {
                text: " please".into()
            },
        ]
    );
}

#[tokio::test]
async fn send_message_stores_a_token_link_segment() {
    use crate::auth::role::ServerRole;
    use crate::data::document::WorldRole;
    use crate::data::sqlite::SqliteRepository;
    use crate::ws::room::RoomRegistry;

    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("W", gm, 0).await.unwrap();
    let ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
    let rate = PingRateLimiter::new();
    let token_id = Uuid::new_v4();

    let (cmd, _pending) = handle_send_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 0,
            budget_per_min: 30,
        },
        "all".into(),
        format!("[[token:{token_id}|Goblin]]"),
        None,
        Audience::Public,
    )
    .await
    .unwrap();
    let doc = match &cmd.ops[0] {
        Operation::Create { doc } => doc,
        other => panic!("expected Create, got {other:?}"),
    };
    let sys: MessageEngine = serde_json::from_value(doc.engine.clone().unwrap()).unwrap();
    assert_eq!(
        sys.content,
        vec![Segment::DocLink {
            target: DocLinkTarget::Token { token_id },
            label: "Goblin".into(),
        }]
    );
}

#[tokio::test]
async fn send_message_with_a_dangling_doc_link_target_still_stores_it_unvalidated() {
    use crate::auth::role::ServerRole;
    use crate::data::document::WorldRole;
    use crate::data::sqlite::SqliteRepository;
    use crate::ws::room::RoomRegistry;

    // No server-side existence check runs against `DocLink`'s target at ingest — a
    // reference to a document that does not exist (or the sender cannot see) is stored
    // verbatim; only the client's render-time `ctx.documents` presence check gates it.
    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("W", gm, 0).await.unwrap();
    let ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
    let rate = PingRateLimiter::new();
    let nonexistent = Uuid::new_v4();

    let (cmd, _pending) = handle_send_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 0,
            budget_per_min: 30,
        },
        "all".into(),
        format!("[[doc:{nonexistent}|Ghost Doc]]"),
        None,
        Audience::Public,
    )
    .await
    .unwrap();
    let doc = match &cmd.ops[0] {
        Operation::Create { doc } => doc,
        other => panic!("expected Create, got {other:?}"),
    };
    let sys: MessageEngine = serde_json::from_value(doc.engine.clone().unwrap()).unwrap();
    assert_eq!(
        sys.content,
        vec![Segment::DocLink {
            target: DocLinkTarget::Doc {
                doc_id: nonexistent,
                embedded_path: None,
            },
            label: "Ghost Doc".into(),
        }]
    );
}

#[tokio::test]
async fn send_message_rejects_a_malformed_doc_link_and_authors_no_message() {
    use crate::auth::role::ServerRole;
    use crate::data::document::WorldRole;
    use crate::data::sqlite::SqliteRepository;
    use crate::ws::room::RoomRegistry;

    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("W", gm, 0).await.unwrap();
    let ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
    let rate = PingRateLimiter::new();
    let seq_before = repo.events_since(w.id, 0).await.unwrap().len();

    let (cmd, _pending) = handle_send_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 0,
            budget_per_min: 30,
        },
        "all".into(),
        "[[doc:not-a-uuid]]".into(),
        None,
        Audience::Public,
    )
    .await
    .unwrap();
    // A malformed doc-link, like any other roll-stage failure, authors ONE whispered
    // System notice instead of the intended message — never both, never neither.
    let doc = match &cmd.ops[0] {
        Operation::Create { doc } => doc,
        other => panic!("expected Create, got {other:?}"),
    };
    assert_eq!(doc.doc_type, MESSAGE_DOC_TYPE);
    let sys: MessageEngine = serde_json::from_value(doc.engine.clone().unwrap()).unwrap();
    assert_eq!(sys.kind, MessageKind::System);
    assert_eq!(
        repo.events_since(w.id, 0).await.unwrap().len(),
        seq_before + 1,
        "exactly one event (the System notice) authored, not the intended message"
    );
}

#[tokio::test]
async fn handle_send_message_rejects_unknown_whisper_recipient() {
    use crate::auth::role::ServerRole;
    use crate::data::document::WorldRole;
    use crate::data::sqlite::SqliteRepository;
    use crate::ws::room::RoomRegistry;

    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = repo
        .create_user("pl", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("W", gm, 0).await.unwrap();
    repo.add_member(w.id, player, WorldRole::Player)
        .await
        .unwrap();
    let ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
    let rate = PingRateLimiter::new();

    // A uuid that belongs to no user at all, let alone this world.
    let foreign = Uuid::from_u128(99_999);
    let err = handle_send_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &super::link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 30,
        },
        "whispers".into(),
        "psst".into(),
        None,
        Audience::Whisper {
            recipients: vec![foreign],
        },
    )
    .await;
    assert!(matches!(err, Err(SendMessageError::UnknownRecipient)));

    // Nothing was persisted — the seq was never consumed.
    assert!(repo.events_since(w.id, 0).await.unwrap().is_empty());
}

#[tokio::test]
async fn handle_send_message_accepts_a_whisper_to_a_real_member() {
    use crate::auth::role::ServerRole;
    use crate::data::document::WorldRole;
    use crate::data::sqlite::SqliteRepository;
    use crate::ws::room::RoomRegistry;

    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = repo
        .create_user("pl", None, ServerRole::User, 0)
        .await
        .unwrap();
    let recipient = repo
        .create_user("re", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("W", gm, 0).await.unwrap();
    repo.add_member(w.id, player, WorldRole::Player)
        .await
        .unwrap();
    repo.add_member(w.id, recipient, WorldRole::Player)
        .await
        .unwrap();
    let ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
    let rate = PingRateLimiter::new();

    let (cmd, _pending) = handle_send_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &super::link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 30,
        },
        "whispers".into(),
        "psst".into(),
        None,
        Audience::Whisper {
            recipients: vec![recipient],
        },
    )
    .await
    .unwrap();
    assert_eq!(cmd.seq, 1);
}

#[tokio::test]
async fn handle_send_message_rejects_oversized_whisper_recipient_list() {
    use crate::auth::role::ServerRole;
    use crate::data::document::WorldRole;
    use crate::data::sqlite::SqliteRepository;
    use crate::ws::room::RoomRegistry;

    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = repo
        .create_user("pl", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("W", gm, 0).await.unwrap();
    repo.add_member(w.id, player, WorldRole::Player)
        .await
        .unwrap();
    let ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
    let rate = PingRateLimiter::new();

    // One over the cap: none of these uuids belong to this world, so if
    // the cap check ran AFTER the per-recipient member_role loop this
    // would instead fail with UnknownRecipient — proving the cap check
    // runs FIRST, before any member_role query.
    let recipients: Vec<Uuid> = (0..(MAX_WHISPER_RECIPIENTS as u128 + 1))
        .map(Uuid::from_u128)
        .collect();
    let err = handle_send_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &super::link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 30,
        },
        "whispers".into(),
        "psst".into(),
        None,
        Audience::Whisper { recipients },
    )
    .await;
    assert!(matches!(err, Err(SendMessageError::TooLong)));
    assert!(
        repo.events_since(w.id, 0).await.unwrap().is_empty(),
        "an oversized whisper must persist nothing"
    );
}

#[tokio::test]
async fn handle_send_message_accepts_whisper_at_exactly_the_recipient_cap() {
    use crate::auth::role::ServerRole;
    use crate::data::document::WorldRole;
    use crate::data::sqlite::SqliteRepository;
    use crate::ws::room::RoomRegistry;

    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = repo
        .create_user("pl", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("W", gm, 0).await.unwrap();
    repo.add_member(w.id, player, WorldRole::Player)
        .await
        .unwrap();
    let ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
    let rate = PingRateLimiter::new();

    // Exactly at the cap, all recipients are the sender themself (a
    // no-op member_role lookup that always succeeds) — this test proves
    // the boundary is accepted, not just that over-the-limit is rejected.
    let recipients: Vec<Uuid> = std::iter::repeat_n(player, MAX_WHISPER_RECIPIENTS).collect();
    let (cmd, _pending) = handle_send_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &super::link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 30,
        },
        "whispers".into(),
        "psst".into(),
        None,
        Audience::Whisper { recipients },
    )
    .await
    .unwrap();
    assert_eq!(cmd.seq, 1);
}

/// A message doc built via `build_message_doc` and committed via
/// `apply_intent` under the posting Player's own ctx (the same write
/// `handle_send_message` performs) is found by ANOTHER world member's
/// `repo.search` — the message rides the existing search index with no
/// message-specific indexing code, and its body text surfaces in the
/// snippet.
#[tokio::test]
async fn source_stores_raw_input_for_plain_and_command_messages() {
    use crate::auth::role::ServerRole;
    use crate::data::document::WorldRole;
    use crate::data::sqlite::SqliteRepository;
    use crate::ws::room::RoomRegistry;

    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = repo
        .create_user("pl", None, ServerRole::User, 0)
        .await
        .unwrap();
    let alice = repo
        .create_user("alice", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("W", gm, 0).await.unwrap();
    repo.add_member(w.id, player, WorldRole::Player)
        .await
        .unwrap();
    repo.add_member(w.id, alice, WorldRole::Player)
        .await
        .unwrap();
    let ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
    let rate = PingRateLimiter::new();

    // Plain message: source == the full content.
    let (cmd, _pending) = handle_send_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &super::link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 30,
        },
        "all".into(),
        "hello".into(),
        None,
        Audience::Public,
    )
    .await
    .unwrap();
    let doc = match &cmd.ops[0] {
        Operation::Create { doc } => doc,
        other => panic!("expected Create, got {other:?}"),
    };
    let sys: MessageEngine = serde_json::from_value(doc.engine.clone().unwrap()).unwrap();
    assert_eq!(sys.source, Some("hello".into()));

    // Command message: source keeps the command prefix (re-parses identically).
    let (cmd, _pending) = handle_send_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &super::link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 101,
            budget_per_min: 30,
        },
        "all".into(),
        "/me waves".into(),
        None,
        Audience::Public,
    )
    .await
    .unwrap();
    let doc = match &cmd.ops[0] {
        Operation::Create { doc } => doc,
        other => panic!("expected Create, got {other:?}"),
    };
    let sys: MessageEngine = serde_json::from_value(doc.engine.clone().unwrap()).unwrap();
    assert_eq!(sys.source, Some("/me waves".into()));

    // Whisper via content /w: source has the /w prefix STRIPPED.
    let (cmd, _pending) = handle_send_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &super::link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 102,
            budget_per_min: 30,
        },
        "all".into(),
        "/w @alice hi".into(),
        None,
        Audience::Public,
    )
    .await
    .unwrap();
    let doc = match &cmd.ops[0] {
        Operation::Create { doc } => doc,
        other => panic!("expected Create, got {other:?}"),
    };
    let sys: MessageEngine = serde_json::from_value(doc.engine.clone().unwrap()).unwrap();
    assert_eq!(sys.source, Some("hi".into()));
}

#[tokio::test]
async fn edit_replaces_source_and_delete_clears_it() {
    use crate::auth::role::ServerRole;
    use crate::data::document::WorldRole;
    use crate::data::sqlite::SqliteRepository;
    use crate::ws::room::RoomRegistry;

    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = repo
        .create_user("pl", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("W", gm, 0).await.unwrap();
    repo.add_member(w.id, player, WorldRole::Player)
        .await
        .unwrap();
    let ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
    let rate = PingRateLimiter::new();

    let (cmd, _pending) = handle_send_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &super::link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 30,
        },
        "all".into(),
        "hello".into(),
        None,
        Audience::Public,
    )
    .await
    .unwrap();
    let message_id = match &cmd.ops[0] {
        Operation::Create { doc } => doc.id,
        other => panic!("expected Create, got {other:?}"),
    };

    handle_edit_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &super::link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 101,
            budget_per_min: 30,
        },
        message_id,
        "goodbye".into(),
    )
    .await
    .unwrap();
    let stored = repo.get_document(message_id).await.unwrap().unwrap();
    let sys: MessageEngine = serde_json::from_value(stored.engine.clone().unwrap()).unwrap();
    assert_eq!(sys.source, Some("goodbye".into()));

    handle_delete_message(&room, &repo, &ctx, &rate, message_id, 102, 30)
        .await
        .unwrap();
    let stored = repo.get_document(message_id).await.unwrap().unwrap();
    let sys: MessageEngine = serde_json::from_value(stored.engine.unwrap()).unwrap();
    assert_eq!(sys.source, None, "delete tombstone must clear source");
    assert!(
        sys.content.is_empty(),
        "delete tombstone must clear content"
    );
}

#[tokio::test]
async fn whisper_edit_prefill_resubmit_is_idempotent() {
    use crate::auth::role::ServerRole;
    use crate::data::document::WorldRole;
    use crate::data::sqlite::SqliteRepository;
    use crate::ws::room::RoomRegistry;

    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let sender = repo
        .create_user("sender", None, ServerRole::User, 0)
        .await
        .unwrap();
    let alice = repo
        .create_user("alice", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("W", gm, 0).await.unwrap();
    repo.add_member(w.id, sender, WorldRole::Player)
        .await
        .unwrap();
    repo.add_member(w.id, alice, WorldRole::Player)
        .await
        .unwrap();
    let ctx = PermissionContext {
        user_id: sender,
        world_role: WorldRole::Player,
    };
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
    let rate = PingRateLimiter::new();

    // Send "/w @alice /me waves": a nested command inside a whisper body is
    // NOT parsed — stored kind is Normal, content/source are the literal
    // post-/w-strip body "/me waves".
    let (cmd, _pending) = handle_send_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &super::link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 30,
        },
        "all".into(),
        "/w @alice /me waves".into(),
        None,
        Audience::Public,
    )
    .await
    .unwrap();
    let message_id = match &cmd.ops[0] {
        Operation::Create { doc } => doc.id,
        other => panic!("expected Create, got {other:?}"),
    };
    let stored = repo.get_document(message_id).await.unwrap().unwrap();
    let sys: MessageEngine = serde_json::from_value(stored.engine.clone().unwrap()).unwrap();
    assert_eq!(sys.kind, MessageKind::Normal);
    assert_eq!(sys.source, Some("/me waves".into()));
    assert!(matches!(sys.audience, Audience::Whisper { .. }));

    // Edit-resubmit of the UNMODIFIED prefill ("/me waves", the stored
    // source): kind/content/source must round-trip unchanged, not reparse
    // into MessageKind::Emote.
    handle_edit_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &super::link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 101,
            budget_per_min: 30,
        },
        message_id,
        "/me waves".into(),
    )
    .await
    .unwrap();
    let stored = repo.get_document(message_id).await.unwrap().unwrap();
    let sys2: MessageEngine = serde_json::from_value(stored.engine.clone().unwrap()).unwrap();
    assert_eq!(
        sys2.kind,
        MessageKind::Normal,
        "must not reparse into Emote"
    );
    assert_eq!(sys2.source, Some("/me waves".into()));
    assert_eq!(sys2.content, sys.content);

    // A second whisper, sent as "/w @alice hi": stored source is the
    // post-strip literal "hi". Edit-resubmitting "hi" (which itself looks
    // like an ordinary /w-free body) must not spuriously trip
    // AudienceLocked — the whole point of skipping command parsing on a
    // whisper edit — and a resubmit of a literal "/w ..." body must also
    // survive without AudienceLocked (only a non-whisper message rejects a
    // literal /w-shaped edit body).
    let (cmd2, _pending2) = handle_send_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &super::link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 102,
            budget_per_min: 30,
        },
        "all".into(),
        "/w @alice hi".into(),
        None,
        Audience::Public,
    )
    .await
    .unwrap();
    let message_id2 = match &cmd2.ops[0] {
        Operation::Create { doc } => doc.id,
        other => panic!("expected Create, got {other:?}"),
    };
    let stored2 = repo.get_document(message_id2).await.unwrap().unwrap();
    let sys2_pre: MessageEngine = serde_json::from_value(stored2.engine.clone().unwrap()).unwrap();
    assert_eq!(sys2_pre.source, Some("hi".into()));

    // Edit-resubmit of a whisper's stored body that itself reads as a /w
    // command must NOT be rejected AudienceLocked — command parsing is
    // skipped entirely on a whisper edit.
    handle_edit_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &super::link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 103,
            budget_per_min: 30,
        },
        message_id2,
        "/w @bob hi".into(),
    )
    .await
    .expect("a whisper edit must never reject a literal /w-shaped body");
    let stored2 = repo.get_document(message_id2).await.unwrap().unwrap();
    let sys2_post: MessageEngine = serde_json::from_value(stored2.engine.unwrap()).unwrap();
    assert_eq!(sys2_post.kind, MessageKind::Normal);
    assert_eq!(sys2_post.source, Some("/w @bob hi".into()));
    // Audience must remain the ORIGINAL whisper's recipients — frozen, not
    // retargeted to @bob, despite the literal body reading as a /w command.
    assert!(
        matches!(sys2_post.audience, Audience::Whisper { ref recipients } if recipients == &vec![alice])
    );

    // Editing a PUBLIC (non-whisper) message with /w-shaped content still
    // rejects AudienceLocked — the fast path applies ONLY to whisper
    // messages, not to every message.
    let (cmd3, _pending3) = handle_send_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &super::link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 104,
            budget_per_min: 30,
        },
        "all".into(),
        "hello".into(),
        None,
        Audience::Public,
    )
    .await
    .unwrap();
    let public_id = match &cmd3.ops[0] {
        Operation::Create { doc } => doc.id,
        other => panic!("expected Create, got {other:?}"),
    };
    let err = handle_edit_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &super::link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 105,
            budget_per_min: 30,
        },
        public_id,
        "/w @alice hi".into(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, SendMessageError::AudienceLocked));

    // An ORDINARY whisper edit (genuinely different content) still
    // sanitizes and updates content/source.
    handle_edit_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &super::link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 106,
            budget_per_min: 30,
        },
        message_id2,
        "bye now".into(),
    )
    .await
    .unwrap();
    let stored2 = repo.get_document(message_id2).await.unwrap().unwrap();
    let sys2_final: MessageEngine = serde_json::from_value(stored2.engine.unwrap()).unwrap();
    assert_eq!(sys2_final.source, Some("bye now".into()));
    match &sys2_final.content[..] {
        [Segment::Text { text }] => assert_eq!(text, "bye now"),
        other => panic!("expected a single Text segment, got {other:?}"),
    }
}

#[tokio::test]
async fn editing_a_normal_message_with_an_inline_roll_segment_is_immutable() {
    // A Normal message whose body embeds an inline roll ("attack!
    // [[1d20]] done") stores kind: Normal (never Roll) but its content
    // still carries a Segment::RollEmbed mid-text. Editing must be
    // rejected the same as editing a top-level `/roll` message would be
    // -- otherwise the roll's audit record could be erased by editing
    // around it.
    use crate::auth::role::ServerRole;
    use crate::data::document::WorldRole;
    use crate::data::sqlite::SqliteRepository;
    use crate::ws::room::RoomRegistry;

    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = repo
        .create_user("pl", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("W", gm, 0).await.unwrap();
    repo.add_member(w.id, player, WorldRole::Player)
        .await
        .unwrap();
    let ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
    let rate = PingRateLimiter::new();

    let (cmd, _pending) = handle_send_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &super::link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 30,
        },
        "all".into(),
        "attack! [[1d20]] done".into(),
        None,
        Audience::Public,
    )
    .await
    .unwrap();
    let (message_id, doc) = match &cmd.ops[0] {
        Operation::Create { doc } => (doc.id, doc),
        other => panic!("expected Create, got {other:?}"),
    };
    let sys: MessageEngine = serde_json::from_value(doc.engine.clone().unwrap()).unwrap();
    assert_eq!(sys.kind, MessageKind::Normal);
    assert!(
        sys.content
            .iter()
            .any(|s| matches!(s, Segment::RollEmbed { .. })),
        "expected an inline RollEmbed segment, got {:?}",
        sys.content
    );

    let err = handle_edit_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &super::link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 101,
            budget_per_min: 30,
        },
        message_id,
        "attack! done, no roll".into(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, SendMessageError::RollImmutable));

    // A plain Normal message (no roll segment) still edits fine.
    let (cmd2, _pending2) = handle_send_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &super::link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 102,
            budget_per_min: 30,
        },
        "all".into(),
        "hello there".into(),
        None,
        Audience::Public,
    )
    .await
    .unwrap();
    let plain_id = match &cmd2.ops[0] {
        Operation::Create { doc } => doc.id,
        other => panic!("expected Create, got {other:?}"),
    };
    handle_edit_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &super::link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 103,
            budget_per_min: 30,
        },
        plain_id,
        "hello again".into(),
    )
    .await
    .expect("a plain Normal message must still edit fine");
}

#[tokio::test]
async fn whisper_roll_via_frame_audience_is_edit_immutable() {
    // `kind == Roll` + `audience == Whisper` IS reachable via the
    // `SendMessage` frame's `audience` field (content has no /w, so parse_command never
    // sets whisper_to, and the frame's Whisper audience is used as-is
    // alongside kind: Roll). The unconditional kind == Roll check must
    // still block editing it.
    use crate::auth::role::ServerRole;
    use crate::data::document::WorldRole;
    use crate::data::sqlite::SqliteRepository;
    use crate::ws::room::RoomRegistry;

    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let sender = repo
        .create_user("sender", None, ServerRole::User, 0)
        .await
        .unwrap();
    let alice = repo
        .create_user("alice", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("W", gm, 0).await.unwrap();
    repo.add_member(w.id, sender, WorldRole::Player)
        .await
        .unwrap();
    repo.add_member(w.id, alice, WorldRole::Player)
        .await
        .unwrap();
    let ctx = PermissionContext {
        user_id: sender,
        world_role: WorldRole::Player,
    };
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
    let rate = PingRateLimiter::new();

    let (cmd, _pending) = handle_send_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &super::link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 30,
        },
        "all".into(),
        "/roll 2d6".into(),
        None,
        Audience::Whisper {
            recipients: vec![alice],
        },
    )
    .await
    .unwrap();
    let (message_id, doc) = match &cmd.ops[0] {
        Operation::Create { doc } => (doc.id, doc),
        other => panic!("expected Create, got {other:?}"),
    };
    let sys: MessageEngine = serde_json::from_value(doc.engine.clone().unwrap()).unwrap();
    assert_eq!(
        sys.kind,
        MessageKind::Roll,
        "expected reachable kind: Roll + audience: Whisper combination"
    );
    assert!(matches!(sys.audience, Audience::Whisper { .. }));

    let err = handle_edit_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &super::link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 101,
            budget_per_min: 30,
        },
        message_id,
        "2d6 edited".into(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, SendMessageError::RollImmutable));
}

#[test]
fn stored_message_without_a_source_key_deserializes() {
    // A stored `MessageEngine` JSON carrying no `source` key at all.
    let j = serde_json::json!({
        "channel": "all",
        "user_owner": Uuid::from_u128(1),
        "kind": "normal",
        "audience": { "kind": "public" },
        "content": [],
    });
    let sys: MessageEngine = serde_json::from_value(j).unwrap();
    assert_eq!(sys.source, None);
}

#[tokio::test]
async fn posted_message_is_searchable_by_members() {
    use crate::auth::role::ServerRole;
    use crate::data::document::WorldRole;
    use crate::data::sqlite::SqliteRepository;

    let r = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = r
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = r
        .create_user("pl", None, ServerRole::User, 0)
        .await
        .unwrap();
    let other = r
        .create_user("ot", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = r.create_world_owned("W", gm, 0).await.unwrap();
    r.add_member(w.id, player, WorldRole::Player).await.unwrap();
    r.add_member(w.id, other, WorldRole::Player).await.unwrap();
    let pl_ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let ot_ctx = PermissionContext {
        user_id: other,
        world_role: WorldRole::Player,
    };

    let doc = build_message_doc(
        w.id,
        player,
        MessageDraft {
            channel: "all".into(),
            actor_owner: None,
            audience: Audience::Public,
            kind: MessageKind::Normal,
            content: plain_text_content("banshee wail"),
            source: None,
        },
        1,
    );
    r.apply_intent(
        &pl_ctx,
        w.id,
        vec![Operation::Create { doc }],
        1,
        WriteOrigin::Client,
    )
    .await
    .unwrap();

    let page = r.search(&ot_ctx, w.id, "banshee", 10, None).await.unwrap();
    assert_eq!(page.hits.len(), 1, "another member finds the message");
    assert!(page.hits[0].snippet.to_lowercase().contains("banshee"));
}

/// A minimal actor doc for the attribution-ownership gate tests, seeded
/// directly via `apply_command` (bypasses permission checks — these
/// tests exercise `handle_send_message`'s attribution gate, not the
/// actor-create authorization path). `engine` must be a well-formed
/// `ActorEngine` body: `apply_command` runs the same `/engine`
/// normalization gate as `apply_intent` (data integrity, not authz),
/// so an absent/malformed body is rejected on Create regardless of
/// this seeding path's relaxed authz.
fn seed_actor_doc(id: Uuid, world: Uuid, owner: Option<Uuid>) -> Document {
    Document {
        id,
        scope: Scope::World { world_id: world },
        doc_type: "actor".into(),
        schema_version: 1,
        name: None,
        source: None,
        base: None,
        owner,
        permissions: crate::data::document::PermissionSet::default(),
        embedded: Default::default(),
        parent_id: None,
        engine: Some(serde_json::json!({
            "displayName": "Goblin",
            "visual": { "kind": "image", "asset": "a.png" },
            "size": { "w": 1.0, "h": 1.0 },
            "shape": "square",
            "faction": null,
            "conditions": [],
            "prototype": true
        })),
        system: serde_json::json!({ "name": "Goblin" }),
        created_at: 0,
        updated_at: 0,
    }
}

/// A minimal token doc for the speak-as-token ingest gate tests, seeded directly via
/// `apply_command` (bypasses permission checks — these tests exercise
/// `handle_send_message`'s attribution gate, not the token-create authorization path).
/// `engine` must be a well-formed `TokenEngine` body, same rationale as `seed_actor_doc`.
fn seed_token_doc(id: Uuid, world: Uuid, owner: Option<Uuid>, actor_id: Option<Uuid>) -> Document {
    Document {
        id,
        scope: Scope::World { world_id: world },
        doc_type: "token".into(),
        schema_version: 1,
        name: None,
        source: None,
        base: None,
        owner,
        permissions: crate::data::document::PermissionSet::default(),
        embedded: Default::default(),
        parent_id: None,
        engine: Some(serde_json::json!({
            "x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0, "rotation": 0.0,
            "actor_id": actor_id,
        })),
        system: serde_json::json!({}),
        created_at: 0,
        updated_at: 0,
    }
}

#[tokio::test]
async fn send_message_allows_token_owner_via_its_own_override_to_speak_as_it() {
    use crate::auth::role::ServerRole;
    use crate::data::command::UnsequencedCommand;
    use crate::data::document::WorldRole;
    use crate::data::sqlite::SqliteRepository;
    use crate::ws::room::RoomRegistry;

    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = repo
        .create_user("pl", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("W", gm, 0).await.unwrap();
    repo.add_member(w.id, player, WorldRole::Player)
        .await
        .unwrap();
    let token_id = Uuid::new_v4();
    repo.apply_command(UnsequencedCommand {
        world_id: w.id,
        author: player,
        ts: 0,
        ops: vec![Operation::Create {
            doc: seed_token_doc(token_id, w.id, Some(player), None),
        }],
    })
    .await
    .unwrap();

    let ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
    let rate = PingRateLimiter::new();

    let (cmd, _pending) = handle_send_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &super::link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 30,
        },
        "all".into(),
        "grr".into(),
        Some(ActorOwnerRef::TokenInstance { token_id }),
        Audience::Public,
    )
    .await
    .unwrap();
    let doc = match &cmd.ops[0] {
        Operation::Create { doc } => doc,
        other => panic!("expected Create, got {other:?}"),
    };
    let sys: MessageEngine = serde_json::from_value(doc.engine.clone().unwrap()).unwrap();
    assert_eq!(
        sys.actor_owner,
        Some(ActorOwnerRef::TokenInstance { token_id })
    );
}

#[tokio::test]
async fn send_message_allows_the_linked_actors_owner_to_speak_as_its_token() {
    use crate::auth::role::ServerRole;
    use crate::data::command::UnsequencedCommand;
    use crate::data::document::WorldRole;
    use crate::data::sqlite::SqliteRepository;
    use crate::ws::room::RoomRegistry;

    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = repo
        .create_user("pl", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("W", gm, 0).await.unwrap();
    repo.add_member(w.id, player, WorldRole::Player)
        .await
        .unwrap();
    let actor_id = Uuid::new_v4();
    let token_id = Uuid::new_v4();
    repo.apply_command(UnsequencedCommand {
        world_id: w.id,
        author: player,
        ts: 0,
        ops: vec![
            Operation::Create {
                doc: seed_actor_doc(actor_id, w.id, Some(player)),
            },
            Operation::Create {
                doc: seed_token_doc(token_id, w.id, None, Some(actor_id)),
            },
        ],
    })
    .await
    .unwrap();

    let ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
    let rate = PingRateLimiter::new();

    let (cmd, _pending) = handle_send_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &super::link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 30,
        },
        "all".into(),
        "grr".into(),
        Some(ActorOwnerRef::TokenInstance { token_id }),
        Audience::Public,
    )
    .await
    .unwrap();
    let doc = match &cmd.ops[0] {
        Operation::Create { doc } => doc,
        other => panic!("expected Create, got {other:?}"),
    };
    let sys: MessageEngine = serde_json::from_value(doc.engine.clone().unwrap()).unwrap();
    assert_eq!(
        sys.actor_owner,
        Some(ActorOwnerRef::TokenInstance { token_id })
    );
}

#[tokio::test]
async fn send_message_rejects_a_non_owner_non_gm_speaking_as_a_token() {
    use crate::auth::role::ServerRole;
    use crate::data::command::UnsequencedCommand;
    use crate::data::document::WorldRole;
    use crate::data::sqlite::SqliteRepository;
    use crate::ws::room::RoomRegistry;

    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = repo
        .create_user("pl", None, ServerRole::User, 0)
        .await
        .unwrap();
    let other = repo
        .create_user("ot", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("W", gm, 0).await.unwrap();
    repo.add_member(w.id, player, WorldRole::Player)
        .await
        .unwrap();
    repo.add_member(w.id, other, WorldRole::Player)
        .await
        .unwrap();
    let token_id = Uuid::new_v4();
    repo.apply_command(UnsequencedCommand {
        world_id: w.id,
        author: other,
        ts: 0,
        ops: vec![Operation::Create {
            doc: seed_token_doc(token_id, w.id, Some(other), None),
        }],
    })
    .await
    .unwrap();

    let ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
    let rate = PingRateLimiter::new();

    let err = handle_send_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &super::link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 30,
        },
        "all".into(),
        "grr".into(),
        Some(ActorOwnerRef::TokenInstance { token_id }),
        Audience::Public,
    )
    .await;
    assert!(matches!(err, Err(SendMessageError::ActorNotSpeakable)));
}

#[tokio::test]
async fn send_message_rejects_a_token_from_another_world_even_for_its_owner() {
    use crate::auth::role::ServerRole;
    use crate::data::command::UnsequencedCommand;
    use crate::data::document::WorldRole;
    use crate::data::sqlite::SqliteRepository;
    use crate::ws::room::RoomRegistry;

    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = repo
        .create_user("pl", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world_a = repo.create_world_owned("A", gm, 0).await.unwrap();
    let world_b = repo.create_world_owned("B", gm, 0).await.unwrap();
    repo.add_member(world_a.id, player, WorldRole::Player)
        .await
        .unwrap();
    repo.add_member(world_b.id, player, WorldRole::Player)
        .await
        .unwrap();

    // The token lives in world B and IS owned by `player` — ownership alone must not be
    // enough to speak as it from world A's room.
    let token_id = Uuid::new_v4();
    repo.apply_command(UnsequencedCommand {
        world_id: world_b.id,
        author: player,
        ts: 0,
        ops: vec![Operation::Create {
            doc: seed_token_doc(token_id, world_b.id, Some(player), None),
        }],
    })
    .await
    .unwrap();

    let ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let reg = RoomRegistry::new();
    let room_a = reg.get_or_create(&repo, world_a.id).await.unwrap().unwrap();
    let rate = PingRateLimiter::new();

    let err = handle_send_message(
        MessageRequestCtx {
            room: &room_a,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &super::link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 0,
            budget_per_min: 30,
        },
        "all".into(),
        "hi".into(),
        Some(ActorOwnerRef::TokenInstance { token_id }),
        Audience::Public,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, SendMessageError::ActorNotSpeakable));
}

#[tokio::test]
async fn send_message_allows_gm_to_speak_as_any_token_regardless_of_owner() {
    use crate::auth::role::ServerRole;
    use crate::data::command::UnsequencedCommand;
    use crate::data::document::WorldRole;
    use crate::data::sqlite::SqliteRepository;
    use crate::ws::room::RoomRegistry;

    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = repo
        .create_user("pl", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("W", gm, 0).await.unwrap();
    repo.add_member(w.id, player, WorldRole::Player)
        .await
        .unwrap();
    let token_id = Uuid::new_v4();
    repo.apply_command(UnsequencedCommand {
        world_id: w.id,
        author: player,
        ts: 0,
        ops: vec![Operation::Create {
            doc: seed_token_doc(token_id, w.id, Some(player), None),
        }],
    })
    .await
    .unwrap();

    let ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
    let rate = PingRateLimiter::new();

    let (cmd, _pending) = handle_send_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &super::link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 30,
        },
        "all".into(),
        "grr".into(),
        Some(ActorOwnerRef::TokenInstance { token_id }),
        Audience::Public,
    )
    .await
    .unwrap();
    let doc = match &cmd.ops[0] {
        Operation::Create { doc } => doc,
        other => panic!("expected Create, got {other:?}"),
    };
    let sys: MessageEngine = serde_json::from_value(doc.engine.clone().unwrap()).unwrap();
    assert_eq!(
        sys.actor_owner,
        Some(ActorOwnerRef::TokenInstance { token_id })
    );
}

#[tokio::test]
async fn send_message_allows_player_attributing_own_actor() {
    use crate::auth::role::ServerRole;
    use crate::data::command::UnsequencedCommand;
    use crate::data::document::WorldRole;
    use crate::data::sqlite::SqliteRepository;
    use crate::ws::room::RoomRegistry;

    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = repo
        .create_user("pl", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("W", gm, 0).await.unwrap();
    repo.add_member(w.id, player, WorldRole::Player)
        .await
        .unwrap();
    let actor_id = Uuid::new_v4();
    repo.apply_command(UnsequencedCommand {
        world_id: w.id,
        author: player,
        ts: 0,
        ops: vec![Operation::Create {
            doc: seed_actor_doc(actor_id, w.id, Some(player)),
        }],
    })
    .await
    .unwrap();

    let ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
    let rate = PingRateLimiter::new();

    let (cmd, _pending) = handle_send_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &super::link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 30,
        },
        "all".into(),
        "grr".into(),
        Some(ActorOwnerRef::Actor { actor_id }),
        Audience::Public,
    )
    .await
    .unwrap();
    let doc = match &cmd.ops[0] {
        Operation::Create { doc } => doc,
        other => panic!("expected Create, got {other:?}"),
    };
    let sys: MessageEngine = serde_json::from_value(doc.engine.clone().unwrap()).unwrap();
    assert_eq!(sys.actor_owner, Some(ActorOwnerRef::Actor { actor_id }));
}

#[tokio::test]
async fn send_message_rejects_player_attributing_another_users_actor() {
    use crate::auth::role::ServerRole;
    use crate::data::command::UnsequencedCommand;
    use crate::data::document::WorldRole;
    use crate::data::sqlite::SqliteRepository;
    use crate::ws::room::RoomRegistry;

    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = repo
        .create_user("pl", None, ServerRole::User, 0)
        .await
        .unwrap();
    let other = repo
        .create_user("ot", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("W", gm, 0).await.unwrap();
    repo.add_member(w.id, player, WorldRole::Player)
        .await
        .unwrap();
    repo.add_member(w.id, other, WorldRole::Player)
        .await
        .unwrap();
    let actor_id = Uuid::new_v4();
    repo.apply_command(UnsequencedCommand {
        world_id: w.id,
        author: other,
        ts: 0,
        ops: vec![Operation::Create {
            doc: seed_actor_doc(actor_id, w.id, Some(other)),
        }],
    })
    .await
    .unwrap();

    let ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
    let rate = PingRateLimiter::new();

    // The seeded actor doc's own Create already consumed one seq;
    // capture it so the assertion below proves the REJECTED send
    // itself persisted nothing, not merely that the log is empty.
    let seq_before = repo.events_since(w.id, 0).await.unwrap().len();
    let err = handle_send_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &super::link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 30,
        },
        "all".into(),
        "grr".into(),
        Some(ActorOwnerRef::Actor { actor_id }),
        Audience::Public,
    )
    .await;
    assert!(matches!(err, Err(SendMessageError::ActorNotSpeakable)));
    assert_eq!(
        repo.events_since(w.id, 0).await.unwrap().len(),
        seq_before,
        "spoofed attribution must persist nothing"
    );
}

#[tokio::test]
async fn actor_from_another_world_is_not_speakable_even_for_its_owner() {
    use crate::auth::role::ServerRole;
    use crate::data::command::UnsequencedCommand;
    use crate::data::document::WorldRole;
    use crate::data::sqlite::SqliteRepository;
    use crate::ws::room::RoomRegistry;

    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = repo
        .create_user("pl", None, ServerRole::User, 0)
        .await
        .unwrap();
    let world_a = repo.create_world_owned("A", gm, 0).await.unwrap();
    let world_b = repo.create_world_owned("B", gm, 0).await.unwrap();
    repo.add_member(world_a.id, player, WorldRole::Player)
        .await
        .unwrap();
    repo.add_member(world_b.id, player, WorldRole::Player)
        .await
        .unwrap();

    // The actor doc lives in world B and IS owned by `player` — ownership
    // alone must not be enough to speak as it from world A's room.
    let actor_id = Uuid::new_v4();
    repo.apply_command(UnsequencedCommand {
        world_id: world_b.id,
        author: player,
        ts: 0,
        ops: vec![Operation::Create {
            doc: seed_actor_doc(actor_id, world_b.id, Some(player)),
        }],
    })
    .await
    .unwrap();

    let ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let reg = RoomRegistry::new();
    let room_a = reg.get_or_create(&repo, world_a.id).await.unwrap().unwrap();
    let rate = PingRateLimiter::new();

    let err = handle_send_message(
        MessageRequestCtx {
            room: &room_a,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &super::link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 0,
            budget_per_min: 30,
        },
        "all".into(),
        "hi".into(),
        Some(ActorOwnerRef::Actor { actor_id }),
        Audience::Public,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, SendMessageError::ActorNotSpeakable));
}

#[tokio::test]
async fn send_message_rejects_attributing_a_nonexistent_actor() {
    use crate::auth::role::ServerRole;
    use crate::data::document::WorldRole;
    use crate::data::sqlite::SqliteRepository;
    use crate::ws::room::RoomRegistry;

    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = repo
        .create_user("pl", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("W", gm, 0).await.unwrap();
    repo.add_member(w.id, player, WorldRole::Player)
        .await
        .unwrap();

    let ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
    let rate = PingRateLimiter::new();

    let err = handle_send_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &super::link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 30,
        },
        "all".into(),
        "grr".into(),
        Some(ActorOwnerRef::Actor {
            actor_id: Uuid::new_v4(),
        }),
        Audience::Public,
    )
    .await;
    assert!(matches!(err, Err(SendMessageError::ActorNotSpeakable)));
}

#[tokio::test]
async fn send_message_allows_gm_attributing_any_actor() {
    use crate::auth::role::ServerRole;
    use crate::data::command::UnsequencedCommand;
    use crate::data::document::WorldRole;
    use crate::data::sqlite::SqliteRepository;
    use crate::ws::room::RoomRegistry;

    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = repo
        .create_user("pl", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("W", gm, 0).await.unwrap();
    repo.add_member(w.id, player, WorldRole::Player)
        .await
        .unwrap();
    let actor_id = Uuid::new_v4();
    repo.apply_command(UnsequencedCommand {
        world_id: w.id,
        author: player,
        ts: 0,
        ops: vec![Operation::Create {
            doc: seed_actor_doc(actor_id, w.id, Some(player)),
        }],
    })
    .await
    .unwrap();

    let ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
    let rate = PingRateLimiter::new();

    let (cmd, _pending) = handle_send_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &super::link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 30,
        },
        "all".into(),
        "grr".into(),
        Some(ActorOwnerRef::Actor { actor_id }),
        Audience::Public,
    )
    .await
    .unwrap();
    // seq 2: the seeded actor doc's own Create consumed seq 1.
    assert_eq!(cmd.seq, 2, "GM may attribute a message to any actor doc");
}

#[tokio::test]
async fn send_message_rejects_attributing_a_nonexistent_token() {
    use crate::auth::role::ServerRole;
    use crate::data::document::WorldRole;
    use crate::data::sqlite::SqliteRepository;
    use crate::ws::room::RoomRegistry;

    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = repo
        .create_user("pl", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("W", gm, 0).await.unwrap();
    repo.add_member(w.id, player, WorldRole::Player)
        .await
        .unwrap();

    let ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
    let rate = PingRateLimiter::new();

    let err = handle_send_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &super::link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 30,
        },
        "all".into(),
        "grr".into(),
        Some(ActorOwnerRef::TokenInstance {
            token_id: Uuid::new_v4(),
        }),
        Audience::Public,
    )
    .await;
    assert!(
        matches!(err, Err(SendMessageError::ActorNotSpeakable)),
        "a token_id with no matching stored document is rejected fail-closed"
    );
}

#[tokio::test]
async fn send_message_rejects_attribution_to_a_non_actor_doc() {
    use crate::auth::role::ServerRole;
    use crate::data::command::UnsequencedCommand;
    use crate::data::document::WorldRole;
    use crate::data::sqlite::SqliteRepository;
    use crate::ws::room::RoomRegistry;

    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = repo
        .create_user("pl", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("W", gm, 0).await.unwrap();
    repo.add_member(w.id, player, WorldRole::Player)
        .await
        .unwrap();
    let mut wrong_type = seed_actor_doc(Uuid::new_v4(), w.id, Some(player));
    wrong_type.doc_type = "note".into();
    // "note" is not engine-defined; a present engine body would now be
    // rejected by apply_command's /engine normalization gate.
    wrong_type.engine = None;
    let doc_id = wrong_type.id;
    repo.apply_command(UnsequencedCommand {
        world_id: w.id,
        author: player,
        ts: 0,
        ops: vec![Operation::Create { doc: wrong_type }],
    })
    .await
    .unwrap();

    let ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
    let rate = PingRateLimiter::new();

    let err = handle_send_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &super::link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 30,
        },
        "all".into(),
        "grr".into(),
        Some(ActorOwnerRef::Actor { actor_id: doc_id }),
        Audience::Public,
    )
    .await;
    assert!(matches!(err, Err(SendMessageError::ActorNotSpeakable)));
}

#[test]
fn wire_recalc_op_converts_to_dice_recalc_op() {
    assert_eq!(
        WireRecalcOp::RerollDice { ids: vec![1, 2] }.into_recalc_op(),
        crate::dice::RecalcOp::RerollDice(vec![1, 2])
    );
    assert_eq!(
        WireRecalcOp::ReplaceDie { id: 3, natural: 5 }.into_recalc_op(),
        crate::dice::RecalcOp::ReplaceDie { id: 3, natural: 5 }
    );
    assert_eq!(
        WireRecalcOp::RemoveDice { ids: vec![4] }.into_recalc_op(),
        crate::dice::RecalcOp::RemoveDice(vec![4])
    );
}

async fn seed_gm_and_room() -> (
    crate::data::sqlite::SqliteRepository,
    std::sync::Arc<crate::ws::room::Room>,
    Uuid,
    Uuid,
    Uuid,
) {
    use crate::auth::role::ServerRole;
    use crate::data::sqlite::SqliteRepository;
    use crate::ws::room::RoomRegistry;

    let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
    let gm = repo
        .create_user("gm", None, ServerRole::User, 0)
        .await
        .unwrap();
    let player = repo
        .create_user("pl", None, ServerRole::User, 0)
        .await
        .unwrap();
    let w = repo.create_world_owned("W", gm, 0).await.unwrap();
    repo.add_member(w.id, player, WorldRole::Player)
        .await
        .unwrap();
    let reg = RoomRegistry::new();
    let room = reg.get_or_create(&repo, w.id).await.unwrap().unwrap();
    (repo, room, w.id, gm, player)
}

#[tokio::test]
async fn handle_recalc_roll_rejects_a_non_gm_sender() {
    let (repo, room, _world, gm, player) = seed_gm_and_room().await;
    let rate = PingRateLimiter::new();
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let (cmd, _pending) = handle_send_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &gm_ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 30,
        },
        "all".into(),
        "/roll 1d6".into(),
        None,
        Audience::Public,
    )
    .await
    .unwrap();
    let doc = match &cmd.ops[0] {
        Operation::Create { doc } => doc.clone(),
        other => panic!("expected Create, got {other:?}"),
    };
    let sys: MessageEngine = serde_json::from_value(doc.engine.unwrap()).unwrap();
    let roll_id = match &sys.content[0] {
        Segment::RollEmbed { roll_id, .. } => *roll_id,
        other => panic!("expected RollEmbed, got {other:?}"),
    };

    let player_ctx = PermissionContext {
        user_id: player,
        world_role: WorldRole::Player,
    };
    let err = handle_recalc_roll(
        RecalcRollRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &player_ctx,
            rate: &rate,
            now: 101,
            budget_per_min: 30,
        },
        doc.id,
        roll_id,
        vec![],
    )
    .await
    .unwrap_err();
    assert!(matches!(err, RecalcRollError::Forbidden));
}

#[tokio::test]
async fn handle_recalc_roll_rejects_unknown_roll_id_and_missing_stored_state() {
    let (repo, room, _world, gm, _player) = seed_gm_and_room().await;
    let rate = PingRateLimiter::new();
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let (cmd, _pending) = handle_send_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &gm_ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 30,
        },
        "all".into(),
        "/roll 1d6".into(),
        None,
        Audience::Public,
    )
    .await
    .unwrap();
    let message_id = match &cmd.ops[0] {
        Operation::Create { doc } => doc.id,
        other => panic!("expected Create, got {other:?}"),
    };

    let err = handle_recalc_roll(
        RecalcRollRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &gm_ctx,
            rate: &rate,
            now: 101,
            budget_per_min: 30,
        },
        message_id,
        Uuid::from_u128(999_999),
        vec![],
    )
    .await
    .unwrap_err();
    assert!(matches!(err, RecalcRollError::RollNotFound));
}

#[tokio::test]
async fn handle_recalc_roll_refuses_a_roll_with_no_stored_spec_or_raw() {
    // A `RollEmbed` seeded directly with its `spec`/`raw` fields both
    // `None` -- a stored document whose roll segment carries neither.
    // Seeded by hand-crafting the stored `engine` JSON rather than going
    // through `handle_send_message` (which always populates both), since
    // that is the only way to construct this shape.
    let (repo, room, world, gm, _player) = seed_gm_and_room().await;
    let rate = PingRateLimiter::new();
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let roll_id = Uuid::from_u128(42);
    let outcome = crate::dice::evaluate(
        &crate::dice::notation::parse(
            "1d6",
            crate::dice::ParseContext {
                mode: crate::dice::notation::ModeKind::Total,
                direction: crate::dice::spec::Direction::HighWins,
            },
        )
        .unwrap(),
        &crate::dice::roll(
            &crate::dice::notation::parse(
                "1d6",
                crate::dice::ParseContext {
                    mode: crate::dice::notation::ModeKind::Total,
                    direction: crate::dice::spec::Direction::HighWins,
                },
            )
            .unwrap(),
            &mut crate::dice::rng::NoiseRng::from_seed(11),
        ),
    );
    let content = vec![Segment::RollEmbed {
        formula: "1d6".into(),
        outcome,
        roll_id,
        spec: None,
        raw: None,
        recalc_history: None,
    }];
    let doc = build_message_doc(
        world,
        gm,
        MessageDraft {
            channel: "all".into(),
            actor_owner: None,
            audience: Audience::Public,
            kind: MessageKind::Roll,
            content,
            source: None,
        },
        0,
    );
    repo.apply_intent(
        &gm_ctx,
        world,
        vec![Operation::Create { doc: doc.clone() }],
        0,
        crate::data::command::WriteOrigin::Client,
    )
    .await
    .unwrap();

    let err = handle_recalc_roll(
        RecalcRollRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &gm_ctx,
            rate: &rate,
            now: 101,
            budget_per_min: 30,
        },
        doc.id,
        roll_id,
        vec![],
    )
    .await
    .unwrap_err();
    assert!(matches!(err, RecalcRollError::NoStoredState));
}

#[tokio::test]
async fn handle_recalc_roll_succeeds_for_public_whisper_and_gmonly_audiences() {
    // Audience-independence (mirrors handle_edit_message/handle_delete_message's
    // own audience-independence tests): a GM's moderation authority to recalc
    // is the same regardless of who can otherwise READ the message.
    for audience in [
        Audience::Public,
        Audience::Whisper { recipients: vec![] },
        Audience::GmOnly,
    ] {
        let (repo, room, _world, gm, _player) = seed_gm_and_room().await;
        let rate = PingRateLimiter::new();
        let gm_ctx = PermissionContext {
            user_id: gm,
            world_role: WorldRole::Gm,
        };
        let (cmd, _pending) = handle_send_message(
            MessageRequestCtx {
                room: &room,
                repo: &repo,
                ctx: &gm_ctx,
                rate: &rate,
                preview: LinkPreviewDeps {
                    client: &link_preview::build_client_allow_loopback(),
                    cache: &LinkPreviewCache::new(),
                    rate: &PreviewRateLimiter::new(),
                },
                now: 100,
                budget_per_min: 30,
            },
            "all".into(),
            "/roll 1d6".into(),
            None,
            audience.clone(),
        )
        .await
        .unwrap();
        let doc = match &cmd.ops[0] {
            Operation::Create { doc } => doc.clone(),
            other => panic!("expected Create, got {other:?}"),
        };
        let sys: MessageEngine = serde_json::from_value(doc.engine.unwrap()).unwrap();
        let roll_id = match &sys.content[0] {
            Segment::RollEmbed { roll_id, .. } => *roll_id,
            other => panic!("expected RollEmbed, got {other:?}"),
        };
        let ok = handle_recalc_roll(
            RecalcRollRequestCtx {
                room: &room,
                repo: &repo,
                ctx: &gm_ctx,
                rate: &rate,
                now: 101,
                budget_per_min: 30,
            },
            doc.id,
            roll_id,
            vec![],
        )
        .await;
        assert!(
            ok.is_ok(),
            "recalc must succeed under {audience:?}, got {ok:?}"
        );
    }
}

#[tokio::test]
async fn handle_recalc_roll_applies_a_reroll_and_appends_recalc_history() {
    let (repo, room, _world, gm, player) = seed_gm_and_room().await;
    let rate = PingRateLimiter::new();
    let gm_ctx = PermissionContext {
        user_id: gm,
        world_role: WorldRole::Gm,
    };
    let (cmd, _pending) = handle_send_message(
        MessageRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &gm_ctx,
            rate: &rate,
            preview: LinkPreviewDeps {
                client: &link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 30,
        },
        "gmonly".into(),
        "/roll 1d6".into(),
        None,
        Audience::GmOnly,
    )
    .await
    .unwrap();
    let doc = match &cmd.ops[0] {
        Operation::Create { doc } => doc.clone(),
        other => panic!("expected Create, got {other:?}"),
    };
    let before: MessageEngine = serde_json::from_value(doc.engine.clone().unwrap()).unwrap();
    let (roll_id, before_raw, before_outcome) = match &before.content[0] {
        Segment::RollEmbed {
            roll_id,
            raw,
            outcome,
            ..
        } => (*roll_id, (**raw.as_ref().unwrap()).clone(), outcome.clone()),
        other => panic!("expected RollEmbed, got {other:?}"),
    };
    let target_id = before_raw.dice[0].id;

    let cmd2 = handle_recalc_roll(
        RecalcRollRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &gm_ctx,
            rate: &rate,
            now: 101,
            budget_per_min: 30,
        },
        doc.id,
        roll_id,
        vec![WireRecalcOp::RerollDice {
            ids: vec![target_id],
        }
        .into_recalc_op()],
    )
    .await
    .unwrap();
    assert_eq!(cmd2.ops.len(), 1);
    let changes = match &cmd2.ops[0] {
        Operation::Update { changes, .. } => changes,
        other => panic!("expected Update, got {other:?}"),
    };
    assert!(changes.iter().any(|c| c.path == "/engine"));
    assert!(changes
        .iter()
        .any(|c| c.path == "/permissions/property_overrides"));

    let stored = repo.get_document(doc.id).await.unwrap().unwrap();
    let after: MessageEngine = serde_json::from_value(stored.engine.unwrap()).unwrap();
    match &after.content[0] {
        Segment::RollEmbed { recalc_history, .. } => {
            let history = recalc_history.as_ref().unwrap();
            assert_eq!(history.len(), 1);
            assert_eq!(history[0].previous_raw, before_raw);
            assert_eq!(history[0].previous_outcome, before_outcome);
            assert_eq!(history[0].recalculated_by, gm);
        }
        other => panic!("expected RollEmbed, got {other:?}"),
    }

    // A second recalc by a GM not individually listed on this GmOnly message
    // still succeeds (moderation authority is audience-independent) and
    // accumulates a SECOND history entry.
    let cmd3 = handle_recalc_roll(
        RecalcRollRequestCtx {
            room: &room,
            repo: &repo,
            ctx: &gm_ctx,
            rate: &rate,
            now: 102,
            budget_per_min: 30,
        },
        doc.id,
        roll_id,
        vec![],
    )
    .await
    .unwrap();
    assert_eq!(cmd3.ops.len(), 1);
    let stored2 = repo.get_document(doc.id).await.unwrap().unwrap();
    let after2: MessageEngine = serde_json::from_value(stored2.engine.clone().unwrap()).unwrap();
    match &after2.content[0] {
        Segment::RollEmbed { recalc_history, .. } => {
            assert_eq!(recalc_history.as_ref().unwrap().len(), 2);
        }
        other => panic!("expected RollEmbed, got {other:?}"),
    }

    // Redaction check: a non-GM's filtered view of the message, AFTER two
    // recalcs, still never contains spec/raw at the top level OR inside
    // EITHER recalc_history entry's previous_raw -- while
    // previous_outcome/recalc_history itself stay visible.
    use crate::data::permission::{filter_properties, resolve_access};
    let player_access = resolve_access(player, WorldRole::Player, &stored2, Some(gm));
    let player_view = filter_properties(&stored2, &player_access).unwrap();
    let player_sys: serde_json::Value = player_view.engine.unwrap();
    let seg = &player_sys["content"][0];
    assert_eq!(seg["spec"], serde_json::Value::Null);
    assert_eq!(seg["raw"], serde_json::Value::Null);
    let history = seg["recalc_history"].as_array().unwrap();
    assert_eq!(
        history.len(),
        2,
        "recalc_history itself is visible to a non-GM"
    );
    for entry in history {
        assert_eq!(
            entry["previous_raw"],
            serde_json::Value::Null,
            "every recalc_history entry's previous_raw must stay gm_only"
        );
        assert!(
            entry.get("previous_outcome").is_some() && !entry["previous_outcome"].is_null(),
            "previous_outcome must stay visible to a non-GM"
        );
    }
}
