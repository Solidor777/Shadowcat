//! Tests that a roll formula nested to the dice parser's cap is stored and read back through
//! every path a message document is read from, at the deepest secret nesting a message holds.

use super::revise_audience_tests::{harness, send};
use super::*;
use crate::data::snapshot::StoredCommand;
use crate::dice::notation::{MAX_EXPR_JSON_DEPTH, MAX_NESTING_DEPTH};

/// A formula nesting `open`/`close` `levels` deep around a `1`.
fn nested(open: &str, close: &str, levels: usize) -> String {
    format!("{}1{}", open.repeat(levels), close.repeat(levels))
}

/// A formula nesting `open`/`close` to the deepest level the parser accepts.
fn deepest(open: &str, close: &str) -> String {
    nested(open, close, MAX_NESTING_DEPTH - 1)
}

/// A flat sum of the most ones the expression depth cap accepts: a left-nested `Expr::Bin` tree
/// one factor deep, two JSON levels per term.
fn chain() -> String {
    vec!["1"; (MAX_EXPR_JSON_DEPTH - 2) / 2 + 1].join("+")
}

/// A body holding `formula` as an inline roll inside secrets nested as deep as the span budget
/// (`rolls::MAX_INLINE_ROLLS`, shared by the roll and every secret) allows.
fn buried(formula: &str) -> String {
    let secrets = rolls::MAX_INLINE_ROLLS - 1;
    format!(
        "{}[[{formula}]]{}",
        "[[secret:gm|".repeat(secrets),
        "]]".repeat(secrets)
    )
}

/// The id of the first roll in `content`, descending into secrets.
fn roll_id_of(content: &[Segment]) -> Option<Uuid> {
    content.iter().find_map(|s| match s {
        Segment::RollEmbed { roll_id, .. } => Some(*roll_id),
        Segment::Secret { content, .. } => roll_id_of(content),
        _ => None,
    })
}

/// Re-reads `events` the way resync does.
fn reread(events: &[StoredCommand]) {
    for ev in events {
        StoredCommand::from_stored_json(&serde_json::to_string(ev).unwrap())
            .expect("a stored event reads back");
    }
}

// Fails if the parser cap lets a stored roll nest past serde_json's recursion limit: the send
// succeeds, then every read of the message row errors and the world's chat cannot load.
#[tokio::test]
async fn a_roll_at_the_nesting_cap_reads_back_through_every_document_path() {
    // Function calls cost three JSON levels each, and `abs(1+...)` five.
    for formula in [
        nested("abs(", ")", (MAX_EXPR_JSON_DEPTH - 2) / 3),
        nested("abs(1+", ")", (MAX_EXPR_JSON_DEPTH - 2) / 5),
        deepest("(", ")"),
        deepest("-", ""),
        chain(),
    ] {
        let h = harness().await;
        let id = send(&h, &h.sender, &buried(&formula), Audience::Public).await;

        let doc = h.repo.get_document(id).await.unwrap().expect("stored");
        assert_eq!(doc.id, id);
        let listed = h
            .repo
            .query_documents(h.room.world_id, MESSAGE_DOC_TYPE)
            .await
            .unwrap();
        assert!(listed.iter().any(|d| d.id == id));

        // A recalculation republishes the whole body inside an Update event.
        let sys: MessageEngine = serde_json::from_value(doc.engine.unwrap()).unwrap();
        let roll_id = roll_id_of(&sys.content);
        let rate = PingRateLimiter::new();
        handle_recalc_roll(
            RecalcRollRequestCtx {
                room: &h.room,
                repo: &h.repo,
                ctx: &h.gm,
                rate: &rate,
                now: 400,
                budget_per_min: 30,
            },
            id,
            roll_id.expect("the body holds a roll"),
            vec![crate::dice::RecalcOp::RerollDice(vec![])],
        )
        .await
        .unwrap();
        h.repo.get_document(id).await.unwrap().expect("re-read");

        let events = h.repo.events_since(h.room.world_id, 0).await.unwrap();
        assert!(events.len() >= 2);
        reread(&events);
    }
}
