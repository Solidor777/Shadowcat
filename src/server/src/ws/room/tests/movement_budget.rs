//! `Room::execute_move`'s combat movement-budget gate: turn-owner enforcement, the resource
//! decrement, and the GM bypass.

use super::*;

/// Drains `rx` until the next `RoomEvent::Event`, returning its `StoredCommand`. Skips any
/// `RoomEvent::Other` (out-of-band aux frames) in between.
async fn next_event(
    rx: &mut tokio::sync::broadcast::Receiver<crate::ws::room::RoomEvent>,
) -> crate::data::snapshot::StoredCommand {
    loop {
        match rx.recv().await.unwrap() {
            crate::ws::room::RoomEvent::Event(ev) => return (*ev).clone(),
            crate::ws::room::RoomEvent::Other(_) => continue,
        }
    }
}

/// `movement_scene("unrestricted", false)` plus an active combat on the same scene: the
/// player's own token is bound to a combatant carrying a `movement` resource; a second
/// (Event-kind, never tied to a token) combatant exists for the turn-ownership tests; a
/// GM-owned token/combatant pair in the SAME combat exercises the GM bypass; and a token NOT
/// bound to any combatant exercises the free-movement case.
struct BudgetHandle {
    inner: MovementHandle,
    combat_id: Uuid,
    combatant_id: Uuid,
    other_combatant: Uuid,
    /// Three king-steps from `start` — one further than `inner.adj2`.
    adj3: (f64, f64),
    /// A path whose total cost exceeds any budget these tests configure, so a Warn/None
    /// enforcement mode that DID truncate would fail the assertion against it.
    long_path: Vec<(f64, f64)>,
    gm_token: Uuid,
    long_path_gm: Vec<(f64, f64)>,
    free_token: Uuid,
    free_start: (f64, f64),
    free_adj: (f64, f64),
}

impl std::ops::Deref for BudgetHandle {
    type Target = MovementHandle;
    fn deref(&self) -> &MovementHandle {
        &self.inner
    }
}

impl BudgetHandle {
    /// Advances the combat's turn to `combatant`. OCC pre-image is the harness's own opening
    /// turn (`combatant_id`) — callers only ever set the turn once per test.
    async fn set_turn(&self, combatant: Uuid) {
        self.room
            .publish(
                &self.repo,
                &self.gm,
                vec![Operation::Update {
                    doc_id: self.combat_id,
                    changes: vec![FieldChange {
                        remove: false,
                        path: "/engine/turn".into(),
                        old: serde_json::json!(self.combatant_id),
                        new: serde_json::json!(combatant),
                    }],
                }],
                0,
                WriteOrigin::Client,
            )
            .await
            .unwrap();
    }

    /// The player's combatant's current `movement` resource value, read off the live ECS
    /// through the same `SceneEcs` seam `Room::execute_move`'s gate uses.
    async fn resource_current(&self) -> f64 {
        let scene = self.room.scene().read().await;
        let (combat_id, _) = scene
            .active_combat_for_scene(self.scene_id)
            .expect("active combat");
        let (_, ce, _, _) = scene
            .combatant_for_token(combat_id, self.token_id)
            .expect("player combatant");
        ce.resources["movement"].current
    }

    /// The GM combatant's current `movement` resource value.
    async fn gm_resource_current(&self) -> f64 {
        let scene = self.room.scene().read().await;
        let (combat_id, _) = scene
            .active_combat_for_scene(self.scene_id)
            .expect("active combat");
        let (_, ce, _, _) = scene
            .combatant_for_token(combat_id, self.gm_token)
            .expect("gm combatant");
        ce.resources["movement"].current
    }

    /// Removes the player's combatant's `movement` resource entry entirely
    /// (`BudgetUnresolvable`'s "no such resource entry" case).
    async fn remove_resource_entry(&self) {
        self.room
            .publish(
                &self.repo,
                &self.gm,
                vec![Operation::Update {
                    doc_id: self.combatant_id,
                    changes: vec![FieldChange {
                        remove: true,
                        path: "/engine/resources/movement".into(),
                        old: serde_json::json!({ "current": 10.0, "max": 30.0 }),
                        new: serde_json::Value::Null,
                    }],
                }],
                0,
                WriteOrigin::Client,
            )
            .await
            .unwrap();
    }
}

/// Builds `movement_scene("unrestricted", false)` plus an active combat: `movement` rules are
/// `{resource: "movement", interpretation, enforcement}`; the scene's `grid.distance` is set to
/// `{perCell: pc, unit: "ft"}` when `per_cell` is `Some`, left absent otherwise. The player's
/// token is bound to a combatant with `{current: 10.0, max: 30.0}` and holds the opening turn.
async fn budget_scene(
    enforcement: &str,
    interpretation: &str,
    per_cell: Option<f64>,
) -> BudgetHandle {
    use crate::data::document::DocRole;
    use serde_json::json;

    let inner = movement_scene("unrestricted", false).await;

    if let Some(pc) = per_cell {
        inner
            .room
            .publish(
                &inner.repo,
                &inner.gm,
                vec![Operation::Update {
                    doc_id: inner.scene_id,
                    changes: vec![FieldChange {
                        remove: false,
                        path: "/engine/grid/distance".into(),
                        old: json!(null),
                        new: json!({ "perCell": pc, "unit": "ft" }),
                    }],
                }],
                0,
                WriteOrigin::Client,
            )
            .await
            .unwrap();
    }

    let wdoc = crate::data::document::tests::world_scoped_doc;
    let (combat_id, combatant_id, other_combatant, gm_token, gm_combatant, free_token) = (
        Uuid::from_u128(0x5CE4),
        Uuid::from_u128(0x5CE5),
        Uuid::from_u128(0x5CE6),
        Uuid::from_u128(0x5CE7),
        Uuid::from_u128(0x5CE8),
        Uuid::from_u128(0x5CE9),
    );

    let mut combat = wdoc(inner.world_id, combat_id, "combat");
    combat.owner = Some(inner.gm.user_id);
    combat.engine = Some(json!({
        "scene_id": inner.scene_id,
        "active": true,
        "round": 1,
        "turn": combatant_id,
        "turn_control": "owner_may_end",
        "order": [combatant_id, other_combatant],
        "movement": {
            "resource": "movement",
            "interpretation": interpretation,
            "enforcement": enforcement
        },
        "effect_cleanup": true,
        "rewind_restore": true,
        "forward_restore": false,
        "effect_lifecycle": {}
    }));
    inner
        .room
        .publish(
            &inner.repo,
            &inner.gm,
            vec![Operation::Create { doc: combat }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

    let mut combatant = wdoc(inner.world_id, combatant_id, "combatant");
    combatant.parent_id = Some(combat_id);
    combatant.owner = Some(inner.player.user_id);
    combatant
        .permissions
        .users
        .insert(inner.player.user_id, DocRole::Owner);
    combatant.engine = Some(json!({
        "kind": { "type": "actor", "token_id": inner.token_id, "actor_id": null },
        "initiative": null,
        "tiebreak": 0.0,
        "resources": { "movement": { "current": 10.0, "max": 30.0 } }
    }));
    inner
        .room
        .publish(
            &inner.repo,
            &inner.gm,
            vec![Operation::Create { doc: combatant }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

    let mut other = wdoc(inner.world_id, other_combatant, "combatant");
    other.parent_id = Some(combat_id);
    other.owner = Some(inner.gm.user_id);
    other.engine = Some(json!({
        "kind": { "type": "event", "lifespan": null, "message": null },
        "initiative": null,
        "tiebreak": 0.0,
        "resources": {}
    }));
    inner
        .room
        .publish(
            &inner.repo,
            &inner.gm,
            vec![Operation::Create { doc: other }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

    let mut gm_tok = wdoc(inner.world_id, gm_token, "token");
    gm_tok.parent_id = Some(inner.scene_id);
    gm_tok.owner = Some(inner.gm.user_id);
    gm_tok.engine = Some(token_engine(50.0, 50.0));
    inner
        .room
        .publish(
            &inner.repo,
            &inner.gm,
            vec![Operation::Create { doc: gm_tok }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

    let mut gm_combatant_doc = wdoc(inner.world_id, gm_combatant, "combatant");
    gm_combatant_doc.parent_id = Some(combat_id);
    gm_combatant_doc.owner = Some(inner.gm.user_id);
    gm_combatant_doc.engine = Some(json!({
        "kind": { "type": "actor", "token_id": gm_token, "actor_id": null },
        "initiative": null,
        "tiebreak": 0.0,
        "resources": { "movement": { "current": 10.0, "max": 30.0 } }
    }));
    inner
        .room
        .publish(
            &inner.repo,
            &inner.gm,
            vec![Operation::Create {
                doc: gm_combatant_doc,
            }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

    let free_start = (50.0, 950.0);
    let mut free_tok = wdoc(inner.world_id, free_token, "token");
    free_tok.parent_id = Some(inner.scene_id);
    free_tok.owner = Some(inner.player.user_id);
    free_tok
        .permissions
        .users
        .insert(inner.player.user_id, DocRole::Owner);
    free_tok.engine = Some(token_engine(free_start.0, free_start.1));
    inner
        .room
        .publish(
            &inner.repo,
            &inner.gm,
            vec![Operation::Create { doc: free_tok }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

    // Straight line of king-steps whose total cost (15 cells) exceeds every budget these tests
    // configure (max 10), so Warn/None's "never truncates" assertion is non-vacuous.
    let long_path: Vec<(f64, f64)> = (0..=15).map(|i| (50.0 + 100.0 * i as f64, 50.0)).collect();
    let long_path_gm = long_path.clone();

    BudgetHandle {
        combat_id,
        combatant_id,
        other_combatant,
        adj3: (350.0, 50.0),
        long_path,
        gm_token,
        long_path_gm,
        free_token,
        free_start,
        free_adj: (150.0, 950.0),
        inner,
    }
}

#[tokio::test]
async fn hard_enforcement_truncates_the_owner_at_the_budget_and_decrements_in_the_same_command() {
    let h = budget_scene("hard", "per_cell", Some(5.0)).await; // 10 ft / 5 ft = 2 cells
    let (mut rx, _) = h.room.subscribe();
    let res = h
        .room
        .execute_move(
            &h.repo,
            &h.player,
            crate::ws::room::MoveRequestInputs {
                scene_id: h.scene_id,
                token: h.token_id,
                path: vec![h.start, h.adj, h.adj2, h.adj3],
                ts: now_millis(),
                request_id: Uuid::nil(),
            },
        )
        .await
        .unwrap();
    assert_eq!(res.stop, h.adj2, "two cells affordable");
    let ev = next_event(&mut rx).await;
    let touched: Vec<Uuid> = ev
        .command
        .ops
        .iter()
        .filter_map(|o| match o {
            Operation::Update { doc_id, .. } => Some(*doc_id),
            _ => None,
        })
        .collect();
    assert!(
        touched.contains(&h.token_id) && touched.contains(&h.combatant_id),
        "position and decrement share the command"
    );
    assert_eq!(h.resource_current().await, 0.0);
}

#[tokio::test]
async fn warn_and_none_never_truncate_but_still_decrement() {
    for mode in ["warn", "none"] {
        let h = budget_scene(mode, "spaces", None).await;
        let res = h
            .room
            .execute_move(
                &h.repo,
                &h.player,
                crate::ws::room::MoveRequestInputs {
                    scene_id: h.scene_id,
                    token: h.token_id,
                    path: h.long_path.clone(),
                    ts: now_millis(),
                    request_id: Uuid::nil(),
                },
            )
            .await
            .unwrap();
        assert_eq!(res.stop, *h.long_path.last().unwrap());
        assert!(h.resource_current().await < 10.0);
    }
}

#[tokio::test]
async fn not_the_turn_owner_is_rejected_under_hard_only() {
    let h = budget_scene("hard", "spaces", None).await;
    h.set_turn(h.other_combatant).await;
    let err = h
        .room
        .execute_move(
            &h.repo,
            &h.player,
            crate::ws::room::MoveRequestInputs {
                scene_id: h.scene_id,
                token: h.token_id,
                path: vec![h.start, h.adj],
                ts: now_millis(),
                request_id: Uuid::nil(),
            },
        )
        .await;
    assert!(matches!(err, Err(DataError::Forbidden)));

    let h = budget_scene("warn", "spaces", None).await;
    h.set_turn(h.other_combatant).await;
    assert!(h
        .room
        .execute_move(
            &h.repo,
            &h.player,
            crate::ws::room::MoveRequestInputs {
                scene_id: h.scene_id,
                token: h.token_id,
                path: vec![h.start, h.adj],
                ts: now_millis(),
                request_id: Uuid::nil(),
            },
        )
        .await
        .is_ok());
}

#[tokio::test]
async fn gm_is_never_truncated_but_is_decremented() {
    let h = budget_scene("hard", "spaces", None).await;
    let res = h
        .room
        .execute_move(
            &h.repo,
            &h.gm,
            crate::ws::room::MoveRequestInputs {
                scene_id: h.scene_id,
                token: h.gm_token,
                path: h.long_path_gm.clone(),
                ts: now_millis(),
                request_id: Uuid::nil(),
            },
        )
        .await
        .unwrap();
    assert_eq!(res.stop, *h.long_path_gm.last().unwrap());
    assert_eq!(h.gm_resource_current().await, 0.0, "floored at zero");
}

#[tokio::test]
async fn missing_per_cell_or_resource_entry_is_refused_with_the_generic_error() {
    let h = budget_scene("hard", "per_cell", None).await;
    assert!(matches!(
        h.room
            .execute_move(
                &h.repo,
                &h.player,
                crate::ws::room::MoveRequestInputs {
                    scene_id: h.scene_id,
                    token: h.token_id,
                    path: vec![h.start, h.adj],
                    ts: now_millis(),
                    request_id: Uuid::nil(),
                },
            )
            .await,
        Err(DataError::Forbidden)
    ));

    let h = budget_scene("hard", "spaces", None).await;
    h.remove_resource_entry().await;
    assert!(matches!(
        h.room
            .execute_move(
                &h.repo,
                &h.player,
                crate::ws::room::MoveRequestInputs {
                    scene_id: h.scene_id,
                    token: h.token_id,
                    path: vec![h.start, h.adj],
                    ts: now_millis(),
                    request_id: Uuid::nil(),
                },
            )
            .await,
        Err(DataError::Forbidden)
    ));
}

#[tokio::test]
async fn a_token_that_is_not_a_combatant_moves_freely_during_combat() {
    let h = budget_scene("hard", "spaces", None).await;
    assert!(h
        .room
        .execute_move(
            &h.repo,
            &h.player,
            crate::ws::room::MoveRequestInputs {
                scene_id: h.scene_id,
                token: h.free_token,
                path: vec![h.free_start, h.free_adj],
                ts: now_millis(),
                request_id: Uuid::nil(),
            },
        )
        .await
        .is_ok());
}
