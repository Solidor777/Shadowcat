//! The session pause's pure classification: every engine document type is classified, and the
//! typed-request predicate names exactly the move request and the eight combat intents.

use super::*;
use crate::data::engine::ENGINE_DOC_TYPES;
use crate::ws::protocol::{CombatRollEntry, ResourceOp};

/// The two lists must name the same document types: a type in `ENGINE_DOC_TYPES` with no entry
/// would default to `Free` silently, and an entry naming no engine type is stale.
#[test]
fn every_engine_doc_type_is_classified() {
    for ty in ENGINE_DOC_TYPES {
        assert!(
            DOC_TYPE_CLASSES.iter().any(|(t, _)| t == ty),
            "engine doc type {ty} has no session-pause class"
        );
    }
    for (ty, _) in DOC_TYPE_CLASSES {
        assert!(
            ENGINE_DOC_TYPES.contains(ty),
            "session-pause class names {ty}, which is not an engine doc type"
        );
    }
    let mut names: Vec<&str> = DOC_TYPE_CLASSES.iter().map(|(t, _)| *t).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(
        names.len(),
        DOC_TYPE_CLASSES.len(),
        "a type is listed twice"
    );
}

/// A token is frozen only for a position write; the combat-clock family is frozen for any write;
/// every other type, including one the engine does not know, is free.
#[test]
fn intent_ops_are_classified_by_doc_type_and_position_write() {
    assert!(blocks_intent_op("token", true));
    assert!(!blocks_intent_op("token", false));
    for ty in [
        "combat",
        "combatant",
        "combatant-group",
        "effect",
        "combat-history",
    ] {
        assert!(blocks_intent_op(ty, false), "{ty} is combat state");
    }
    for ty in [
        "scene",
        "wall",
        "region",
        "message",
        "actor",
        "note",
        "item",
        "world-settings",
    ] {
        assert!(!blocks_intent_op(ty, true), "{ty} stays writable");
    }
}

/// The typed-request predicate names the move request and each combat intent, and nothing the
/// owner's ruling leaves open: doors, chat, dice, pings and pathfinding reads.
#[test]
fn blocks_msg_names_the_move_and_combat_requests_only() {
    let id = Uuid::nil();
    let blocked = [
        ClientMsg::MoveRequest {
            request_id: id,
            scene: id,
            token_id: id,
            path: Vec::new(),
        },
        ClientMsg::CombatStart {
            request_id: id,
            combat_id: id,
        },
        ClientMsg::CombatPause {
            request_id: id,
            combat_id: id,
        },
        ClientMsg::CombatEnd {
            request_id: id,
            combat_id: id,
        },
        ClientMsg::CombatAdvance {
            request_id: id,
            combat_id: id,
        },
        ClientMsg::CombatRewind {
            request_id: id,
            combat_id: id,
        },
        ClientMsg::CombatSort {
            request_id: id,
            combat_id: id,
        },
        ClientMsg::CombatRoll {
            request_id: id,
            combat_id: id,
            channel: "main".into(),
            rolls: vec![CombatRollEntry {
                combatant_id: id,
                notation: "1d20".into(),
            }],
        },
        ClientMsg::CombatResource {
            request_id: id,
            combat_id: id,
            combatant_id: id,
            resource: "hp".into(),
            op: ResourceOp::Delta { amount: -1.0 },
        },
    ];
    for m in &blocked {
        assert!(blocks_msg(m), "{m:?} is a move or combat control");
        assert!(paused_reply(m).is_some());
    }
    let free = [
        ClientMsg::ToggleDoor {
            request_id: id,
            scene_id: id,
            wall_id: id,
            to: crate::data::engine::DoorState::Open,
        },
        ClientMsg::Pong,
        ClientMsg::ScenePing {
            scene: id,
            x: 0.0,
            y: 0.0,
        },
    ];
    for m in &free {
        assert!(!blocks_msg(m), "{m:?} stays allowed while paused");
        assert!(paused_reply(m).is_none());
    }
}

/// A move is answered with `MoveError`, a combat intent with `CombatError`; both carry the
/// request's own correlation id and the paused reason.
#[test]
fn paused_replies_are_correlated_and_typed() {
    let id = Uuid::from_u128(7);
    let mv = paused_reply(&ClientMsg::MoveRequest {
        request_id: id,
        scene: Uuid::nil(),
        token_id: Uuid::nil(),
        path: Vec::new(),
    });
    assert!(matches!(
        mv,
        Some(ServerMsg::MoveError { request_id, reason: Some(RejectReason::Paused), .. })
            if request_id == id
    ));
    let combat = paused_reply(&ClientMsg::CombatAdvance {
        request_id: id,
        combat_id: Uuid::nil(),
    });
    assert!(matches!(
        combat,
        Some(ServerMsg::CombatError { request_id, reason: Some(RejectReason::Paused), .. })
            if request_id == id
    ));
}
