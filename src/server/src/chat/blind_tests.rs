//! Tests for `Audience::Blind` and the `/selfroll` whisper: which reader receives a roll's
//! formula and result, the refusals, recalculation and audience revision over a blind roll, and
//! the search partitions.

use super::revise_audience_tests::{harness, next_event, send, wire_ops, Harness};
use super::*;
use crate::data::command::Operation;
use crate::data::document::DocRole;
use crate::data::membership::PermissionContext;
use crate::data::permission::{filter_properties, resolve_access};
use crate::data::search::{index_content_gm, index_content_public};
use crate::ws::PingRateLimiter;

/// Sends `content` as `who` with the frame audience `audience`, surfacing the ingest error.
async fn try_send(
    h: &Harness,
    who: &PermissionContext,
    content: &str,
    audience: Audience,
) -> Result<Option<(Command, Vec<PendingEnrichment>)>, SendMessageError> {
    let rate = PingRateLimiter::new();
    handle_send_message(
        MessageRequestCtx {
            room: &h.room,
            repo: &h.repo,
            ctx: who,
            rate: &rate,
            vfx_rate: &rate,
            preview: LinkPreviewDeps {
                client: &link_preview::build_client_allow_loopback(),
                cache: &LinkPreviewCache::new(),
                rate: &PreviewRateLimiter::new(),
            },
            now: 100,
            budget_per_min: 30,
        },
        "general".into(),
        content.into(),
        None,
        audience,
        None,
    )
    .await
}

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

/// The engine body of the message `id` as `who` receives it in the broadcast `ev`.
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

/// The engine body of the stored message `id` as `who` reads it from a list: the stored
/// document filtered by that reader's access.
async fn listed_engine(h: &Harness, who: &PermissionContext, id: Uuid) -> serde_json::Value {
    let doc = h.repo.get_document(id).await.unwrap().unwrap();
    let access = resolve_access(who.user_id, who.world_role, &doc, doc.owner);
    filter_properties(&doc, &access).unwrap().engine.unwrap()
}

/// A request context for a revision or recalculation by `who`.
fn req<'a>(
    h: &'a Harness,
    who: &'a PermissionContext,
    rate: &'a PingRateLimiter,
) -> RecalcRollRequestCtx<'a> {
    RecalcRollRequestCtx {
        room: &h.room,
        repo: &h.repo,
        ctx: who,
        rate,
        now: 200,
        budget_per_min: 30,
    }
}

/// Asserts a roll segment of a delivered body carries neither formula, outcome nor recalc
/// outcome, and that `source` is gone.
fn assert_blinded(engine: &serde_json::Value) {
    let seg = &engine["content"][0];
    assert_eq!(seg["kind"], "roll_embed");
    assert!(seg.get("formula").is_none(), "formula leaked: {seg}");
    assert!(seg.get("outcome").is_none(), "outcome leaked: {seg}");
    assert!(engine.get("source").is_none(), "source leaked: {engine}");
    assert!(
        !engine.to_string().contains("7d11"),
        "the formula appears somewhere in the body: {engine}"
    );
}

// Fails if `push_segment_overrides` stops registering `formula`/`outcome` for a blind roll: the
// roller's copy then carries both.
#[tokio::test]
async fn a_blind_roll_reaches_only_the_gm_with_its_formula_and_result() {
    let h = harness().await;
    let (id, ev) = send_and_event(&h, &h.sender, "/blindroll 7d11 | sneaking").await;

    let gm = delivered_engine(&h, &h.gm, &ev, id).await;
    assert_eq!(gm["audience"]["kind"], "blind");
    assert_eq!(gm["kind"], "roll");
    assert_eq!(gm["content"][0]["formula"], "7d11");
    assert!(gm["content"][0]["outcome"]["total"].is_number());
    assert_eq!(gm["source"], "7d11 | sneaking");

    for who in [&h.sender, &h.bystander, &h.friend] {
        let copy = delivered_engine(&h, who, &ev, id).await;
        assert_blinded(&copy);
        assert_eq!(copy["audience"]["kind"], "blind");
        assert_eq!(copy["content"][0]["flavor"], "sneaking");
        // The redacted copy still parses as a message body.
        serde_json::from_value::<MessageEngine>(copy).expect("a redacted blind copy deserializes");
    }
}

// The same hiding on the list path (the resync and load read), not only the broadcast.
#[tokio::test]
async fn a_blind_roll_is_hidden_from_a_non_gm_list_and_shown_to_the_gm() {
    let h = harness().await;
    let id = send(&h, &h.sender, "/br 7d11", Audience::Public).await;
    assert!(listed_engine(&h, &h.gm, id).await["content"][0]["outcome"].is_object());
    for who in [&h.sender, &h.bystander] {
        assert_blinded(&listed_engine(&h, who, id).await);
    }
}

// Control: off the opt-in path the roller reads their own roll, formula, outcome and source.
#[tokio::test]
async fn a_plain_roll_still_shows_the_roller_the_result() {
    let h = harness().await;
    let (id, ev) = send_and_event(&h, &h.sender, "/roll 7d11").await;
    let copy = delivered_engine(&h, &h.sender, &ev, id).await;
    assert_eq!(copy["content"][0]["formula"], "7d11");
    assert!(copy["content"][0]["outcome"]["total"].is_number());
    assert_eq!(copy["source"], "/roll 7d11");
}

#[tokio::test]
async fn a_selfroll_reaches_the_roller_alone_and_keeps_them_owner() {
    let h = harness().await;
    for command in ["/selfroll 1d20", "/sr 1d20"] {
        let (id, ev) = send_and_event(&h, &h.sender, command).await;
        let stored = h.repo.get_document(id).await.unwrap().unwrap();
        let engine: MessageEngine = serde_json::from_value(stored.engine.clone().unwrap()).unwrap();
        assert_eq!(engine.kind, MessageKind::Roll);
        assert_eq!(
            engine.audience,
            Audience::Whisper {
                recipients: Vec::new()
            }
        );
        assert_eq!(
            stored.permissions.users.get(&h.sender.user_id),
            Some(&DocRole::Owner)
        );
        assert_eq!(stored.permissions.users.len(), 1);
        assert_eq!(stored.permissions.default, DocRole::None);
        assert_eq!(stored.permissions.gm_role, Some(DocRole::None));

        let mine = wire_ops(&h, &h.sender, &ev).await;
        assert!(mine.iter().any(
            |op| matches!(op, Operation::Create { doc } if doc.id == id && doc.engine.is_some())
        ));
        // The author's own copy keeps the result.
        assert!(
            delivered_engine(&h, &h.sender, &ev, id).await["content"][0]["outcome"].is_object()
        );
        for who in [&h.gm, &h.friend, &h.bystander] {
            let others = wire_ops(&h, who, &ev).await;
            assert!(
                !others.iter().any(
                    |op| matches!(op, Operation::Create { doc } if doc.id == id && doc.engine.is_some())
                ),
                "{command}: delivered to a reader the whisper excludes"
            );
        }
    }
}

#[tokio::test]
async fn a_blind_message_with_no_roll_is_refused_and_persists_nothing() {
    let h = harness().await;
    let (_, seq_before) = h.room.subscribe();
    let err = try_send(&h, &h.sender, "hello", Audience::Blind)
        .await
        .unwrap_err();
    assert!(matches!(err, SendMessageError::BlindNeedsRoll), "{err:?}");
    assert_eq!(err.to_string(), "A blind message needs a roll to hide.");
    assert_eq!(h.room.subscribe().1, seq_before, "nothing was published");
}

// A `/w` after the command is part of the roll formula, so it fails as a roll and the sender gets
// the whispered error notice: no blind message is authored.
#[tokio::test]
async fn blindroll_cannot_carry_a_whisper_prefix() {
    let h = harness().await;
    let (id, _) = send_and_event(&h, &h.sender, "/blindroll /w @friend 1d20").await;
    let stored = h.repo.get_document(id).await.unwrap().unwrap();
    let engine: MessageEngine = serde_json::from_value(stored.engine.unwrap()).unwrap();
    assert_eq!(engine.kind, MessageKind::System);
    assert_ne!(engine.audience, Audience::Blind);
}

// A command audience wins over the frame's audience, as `/gmroll` and `/publicroll` do.
#[tokio::test]
async fn blindroll_overrides_a_whisper_frame_audience() {
    let h = harness().await;
    let (cmd, _) = try_send(
        &h,
        &h.sender,
        "/blindroll 1d20",
        Audience::Whisper {
            recipients: vec![h.friend.user_id],
        },
    )
    .await
    .unwrap()
    .unwrap();
    let Operation::Create { doc } = &cmd.ops[0] else {
        unreachable!()
    };
    let engine: MessageEngine = serde_json::from_value(doc.engine.clone().unwrap()).unwrap();
    assert_eq!(engine.audience, Audience::Blind);
}

/// The stored message's roll id.
async fn roll_id_of(h: &Harness, id: Uuid) -> Uuid {
    let stored = h.repo.get_document(id).await.unwrap().unwrap();
    let engine: MessageEngine = serde_json::from_value(stored.engine.unwrap()).unwrap();
    match &engine.content[0] {
        Segment::RollEmbed { roll_id, .. } => *roll_id,
        other => panic!("expected a roll, got {other:?}"),
    }
}

#[tokio::test]
async fn the_roller_cannot_recalculate_a_blind_roll_and_the_gm_can() {
    let h = harness().await;
    let id = send(&h, &h.sender, "/blindroll 7d11", Audience::Public).await;
    let roll_id = roll_id_of(&h, id).await;
    let rate = PingRateLimiter::new();

    let err = handle_recalc_roll(
        RecalcRollRequestCtx {
            room: &h.room,
            repo: &h.repo,
            ctx: &h.sender,
            rate: &rate,
            now: 200,
            budget_per_min: 30,
        },
        id,
        roll_id,
        vec![crate::dice::RecalcOp::RerollDice(vec![])],
    )
    .await
    .unwrap_err();
    assert!(matches!(err, RecalcRollError::Forbidden), "{err:?}");

    handle_recalc_roll(
        req(&h, &h.gm, &rate),
        id,
        roll_id,
        vec![crate::dice::RecalcOp::RerollDice(vec![])],
    )
    .await
    .expect("the GM recalculates a blind roll");

    // The recalculation rebuilt the overrides through the same decider: the history entry's
    // previous outcome is hidden along with the roll's own, and the stored set equals a fresh
    // computation over the stored body.
    let stored = h.repo.get_document(id).await.unwrap().unwrap();
    let engine: MessageEngine = serde_json::from_value(stored.engine.clone().unwrap()).unwrap();
    assert_eq!(
        stored.permissions.property_overrides,
        message_property_overrides(&engine)
    );
    for key in [
        "/engine/content/0/formula",
        "/engine/content/0/outcome",
        "/engine/content/0/recalc_history/0/previous_outcome",
        "/engine/content/0/recalc_history/0/previous_raw",
        "/engine/source",
    ] {
        assert!(
            stored.permissions.property_overrides.contains_key(key),
            "missing {key}"
        );
    }
    let copy = listed_engine(&h, &h.sender, id).await;
    assert_blinded(&copy);
    assert!(
        copy["content"][0]["recalc_history"][0]
            .get("previous_outcome")
            .is_none(),
        "the roller learns the pre-recalculation result: {copy}"
    );
    serde_json::from_value::<MessageEngine>(copy).expect("a recalculated blind copy deserializes");
    let gm_copy = listed_engine(&h, &h.gm, id).await;
    assert!(gm_copy["content"][0]["recalc_history"][0]["previous_outcome"].is_object());
}

// Fails if `handle_revise_audience` computes the overrides before storing the new audience: a
// revision to `Blind` then keeps the old, open override set.
#[tokio::test]
async fn revising_a_roll_to_blind_hides_it_and_revealing_it_shows_it_again() {
    let h = harness().await;
    let id = send(&h, &h.sender, "/roll 7d11", Audience::Public).await;
    assert!(listed_engine(&h, &h.sender, id).await["content"][0]["outcome"].is_object());
    let rate = PingRateLimiter::new();

    handle_revise_audience(req(&h, &h.gm, &rate), id, Audience::Blind)
        .await
        .unwrap();
    assert_blinded(&listed_engine(&h, &h.sender, id).await);
    assert!(listed_engine(&h, &h.gm, id).await["content"][0]["outcome"].is_object());

    handle_revise_audience(req(&h, &h.gm, &rate), id, Audience::Public)
        .await
        .unwrap();
    let revealed = listed_engine(&h, &h.sender, id).await;
    assert_eq!(revealed["content"][0]["formula"], "7d11");
    assert!(revealed["content"][0]["outcome"]["total"].is_number());
    assert_eq!(revealed["source"], "/roll 7d11");
}

#[tokio::test]
async fn revising_a_message_with_no_roll_to_blind_is_refused() {
    let h = harness().await;
    let id = send(&h, &h.sender, "just words", Audience::Public).await;
    let rate = PingRateLimiter::new();
    let err = handle_revise_audience(req(&h, &h.gm, &rate), id, Audience::Blind)
        .await
        .unwrap_err();
    assert!(matches!(err, SendMessageError::BlindNeedsRoll), "{err:?}");
    let stored = h.repo.get_document(id).await.unwrap().unwrap();
    assert!(stored.permissions.property_overrides.is_empty());
}

// The public partition holds the roll's public text (its flavor) and never its formula; the GM
// partition holds both.
#[tokio::test]
async fn a_blind_rolls_formula_is_indexed_for_the_gm_only() {
    let h = harness().await;
    let id = send(
        &h,
        &h.sender,
        "/blindroll 7d11 | xylophone",
        Audience::Public,
    )
    .await;
    let stored = h.repo.get_document(id).await.unwrap().unwrap();

    let public = index_content_public(&stored);
    assert!(public.contains("xylophone"), "flavor is public: {public}");
    assert!(!public.contains("7d11"), "formula leaked: {public}");
    let gm = index_content_gm(&stored);
    assert!(gm.contains("xylophone") && gm.contains("7d11"), "{gm}");
}

#[test]
fn the_blind_rule_registers_a_table_draws_formula_outcome_and_row() {
    let nested = TableDrawSegment {
        table_id: Uuid::nil(),
        table_name: "inner".into(),
        roll_id: Uuid::nil(),
        formula: "1d2".into(),
        outcome: crate::dice::RollOutcome::default(),
        spec: None,
        raw: None,
        row: None,
    };
    let draw = Segment::TableDraw(TableDrawSegment {
        table_id: Uuid::nil(),
        table_name: "loot".into(),
        roll_id: Uuid::nil(),
        formula: "1d6".into(),
        outcome: crate::dice::RollOutcome::default(),
        spec: None,
        raw: None,
        row: Some(DrawnRow {
            index: 0,
            label: "coin".into(),
            content: Vec::new(),
            nested: vec![nested],
        }),
    });
    let content = [draw];

    let open = roll_property_overrides(&content, &Audience::Public);
    assert!(
        !open.contains_key("/engine/content/0/row"),
        "an open draw hides nothing: {open:?}"
    );

    let blind = roll_property_overrides(&content, &Audience::Blind);
    let keys: Vec<&str> = blind.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "/engine/content/0/formula",
            "/engine/content/0/outcome",
            "/engine/content/0/row"
        ]
    );
    assert!(blind.values().all(|v| *v == Visibility::GmOnly));
}

// The note engine builds its overrides through `segment_property_overrides`, which never hides
// results; a blind switch must not leak into it.
#[test]
fn segment_property_overrides_never_applies_the_blind_rule() {
    let roll = Segment::RollEmbed {
        formula: "1d6".into(),
        outcome: crate::dice::RollOutcome::default(),
        roll_id: Uuid::nil(),
        spec: None,
        raw: None,
        recalc_history: None,
        flavor: None,
    };
    assert!(segment_property_overrides("/engine/body", std::slice::from_ref(&roll)).is_empty());
    assert!(!roll_property_overrides(std::slice::from_ref(&roll), &Audience::Blind).is_empty());
    assert!(roll_property_overrides(std::slice::from_ref(&roll), &Audience::GmOnly).is_empty());
}

#[test]
fn blind_has_the_public_readership_shape() {
    let author = Uuid::new_v4();
    let (default, gm_role, users) = permissions_for_audience(author, &Audience::Blind);
    assert_eq!(default, DocRole::Observer);
    assert_eq!(gm_role, None);
    assert_eq!(users.get(&author), Some(&DocRole::Owner));
    assert_eq!(
        serde_json::to_value(Audience::Blind).unwrap(),
        serde_json::json!({ "kind": "blind" })
    );
}
