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
    gm_combatant: Uuid,
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

    /// Turns the player's combatant into one the player has NO relationship to and cannot read:
    /// `permissions.default: none`, no per-user grant, no `owner`. The player still owns the
    /// TOKEN, so they can still ask to move it — the exact reachability the hidden-combatant
    /// secrecy rule covers, and the shape `SceneEcs::combatant_for_token`'s `actor_id` fallback
    /// also produces for any token instanced from the same actor.
    async fn hide_player_combatant(&self) {
        let doc = self
            .repo
            .get_document(self.combatant_id)
            .await
            .unwrap()
            .expect("player combatant");
        let mut hidden = doc.permissions.clone();
        hidden.default = crate::data::document::DocRole::None;
        hidden.users.clear();
        self.room
            .publish(
                &self.repo,
                &self.gm,
                vec![Operation::Update {
                    doc_id: self.combatant_id,
                    changes: vec![
                        FieldChange {
                            remove: false,
                            path: "/permissions".into(),
                            old: serde_json::to_value(&doc.permissions).unwrap(),
                            new: serde_json::to_value(&hidden).unwrap(),
                        },
                        FieldChange {
                            remove: false,
                            path: "/owner".into(),
                            old: serde_json::json!(doc.owner),
                            new: serde_json::Value::Null,
                        },
                    ],
                }],
                0,
                WriteOrigin::Client,
            )
            .await
            .unwrap();
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

    /// Removes the GM's own combatant's `movement` resource entry entirely — the GM-exemption
    /// counterpart to `remove_resource_entry`.
    async fn remove_gm_resource_entry(&self) {
        self.room
            .publish(
                &self.repo,
                &self.gm,
                vec![Operation::Update {
                    doc_id: self.gm_combatant,
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
        gm_combatant,
        long_path_gm,
        free_token,
        free_start,
        free_adj: (150.0, 950.0),
        inner,
    }
}

#[tokio::test]
async fn hard_enforcement_truncates_the_owner_at_the_budget_and_decrements_via_a_separate_command()
{
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

    // The position write commits FIRST, alone — it must never carry the decrement in the same
    // command (that bundling is the authorization bypass this fix closes: `apply_intent` skips
    // the ownership check for every op in a batch whenever any op carries `CombatTransition`).
    let position_ev = next_event(&mut rx).await;
    let position_touched: Vec<Uuid> = position_ev
        .command
        .ops
        .iter()
        .filter_map(|o| match o {
            Operation::Update { doc_id, .. } => Some(*doc_id),
            _ => None,
        })
        .collect();
    assert_eq!(
        position_touched,
        vec![h.token_id],
        "position write commits alone under Client, never bundled with the decrement"
    );

    // The budget decrement commits SEPARATELY, as its own command, touching only the combatant.
    let decrement_ev = next_event(&mut rx).await;
    let decrement_touched: Vec<Uuid> = decrement_ev
        .command
        .ops
        .iter()
        .filter_map(|o| match o {
            Operation::Update { doc_id, .. } => Some(*doc_id),
            _ => None,
        })
        .collect();
    assert_eq!(
        decrement_touched,
        vec![h.combatant_id],
        "decrement commits as its own command, touching only the combatant"
    );
    assert!(
        decrement_ev.command.seq > position_ev.command.seq,
        "decrement's command sequences strictly after the position commit"
    );

    assert_eq!(h.resource_current().await, 0.0, "floored at zero");
    assert_eq!(
        h.committed_pos(h.token_id).await,
        h.adj2,
        "position truncated at the budget ceiling"
    );
}

#[tokio::test]
async fn a_non_owner_cannot_move_another_players_token_during_combat() {
    let h = budget_scene("hard", "spaces", None).await;

    // A second, unrelated player with no ownership grant on `h.token_id` — the token belongs
    // exclusively to `h.player` (`token.permissions.users.insert(p, DocRole::Owner)` in
    // `movement_scene_with_speed`). This player is a plain world member with nothing on it.
    let intruder_id = h
        .repo
        .create_user("intruder", None, ServerRole::User, 0)
        .await
        .unwrap();
    h.repo
        .add_member(h.world_id, intruder_id, WorldRole::Player)
        .await
        .unwrap();
    let intruder = PermissionContext {
        user_id: intruder_id,
        world_role: WorldRole::Player,
    };

    let err = h
        .room
        .execute_move(
            &h.repo,
            &intruder,
            crate::ws::room::MoveRequestInputs {
                scene_id: h.scene_id,
                token: h.token_id,
                path: vec![h.start, h.adj],
                ts: now_millis(),
                request_id: Uuid::nil(),
            },
        )
        .await;
    assert!(
        matches!(err, Err(DataError::Forbidden)),
        "a non-owner must be refused by the ordinary ownership check during active combat, \
         exactly as outside of it — regression test for the CombatTransition-bundling bypass"
    );
    assert_eq!(
        h.committed_pos(h.token_id).await,
        h.start,
        "the token must not have moved"
    );
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

/// A combatant the mover cannot READ must not gate their move at all: neither the turn-owner
/// refusal (which would disclose the combatant's existence) nor the truncation (which would
/// disclose its exact numeric budget) may apply. Both halves are asserted against the SAME
/// configurations their enforced counterparts (`not_the_turn_owner_is_rejected_under_hard_only`,
/// `hard_enforcement_truncates_the_owner_at_the_budget_...`) use, so the difference under test
/// is the combatant's readability and nothing else.
#[tokio::test]
async fn a_hidden_combatant_neither_refuses_nor_truncates_a_mover_who_cannot_read_it() {
    // Turn-owner refusal: identical to `not_the_turn_owner_is_rejected_under_hard_only`'s Hard
    // case, which refuses — hiding the combatant must turn that into an ordinary move.
    let h = budget_scene("hard", "spaces", None).await;
    h.hide_player_combatant().await;
    h.set_turn(h.other_combatant).await;
    assert!(
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
            .await
            .is_ok(),
        "a refusal here would disclose that the token is bound to a hidden combatant"
    );

    // Truncation: a 15-cell path against a 10-space budget. An enforced mover stops at 10;
    // this one must reach the end, disclosing no budget value through the stop position.
    let h = budget_scene("hard", "spaces", None).await;
    h.hide_player_combatant().await;
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
    assert_eq!(
        res.stop,
        *h.long_path.last().unwrap(),
        "truncation would disclose the hidden combatant's exact budget"
    );
    // The spend is still recorded on the hidden combatant's own document — `filter_command`
    // drops that whole Update for any recipient without READ on it, so recording it leaks
    // nothing while keeping the budget honest for the GM who can see it.
    assert_eq!(
        h.resource_current().await,
        0.0,
        "decrement still applied, floored at zero"
    );
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
async fn gm_with_an_unresolvable_budget_moves_freely_with_no_decrement() {
    // GM's combatant carries no `movement` resource entry at all — the non-GM equivalent of
    // this is `missing_per_cell_or_resource_entry_is_refused_with_the_generic_error`'s
    // `Forbidden`. For a GM this must degrade to "move freely, no decrement" instead.
    let h = budget_scene("hard", "spaces", None).await;
    h.remove_gm_resource_entry().await;
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
    assert_eq!(
        res.stop,
        *h.long_path_gm.last().unwrap(),
        "GM moves the full path — no truncation from an unresolvable budget"
    );

    // GM's combatant carries `PerCell` interpretation with no `grid.distance` configured — the
    // non-GM equivalent is the `per_cell`/`None` case in
    // `missing_per_cell_or_resource_entry_is_refused_with_the_generic_error`.
    let h2 = budget_scene("hard", "per_cell", None).await;
    let res2 = h2
        .room
        .execute_move(
            &h2.repo,
            &h2.gm,
            crate::ws::room::MoveRequestInputs {
                scene_id: h2.scene_id,
                token: h2.gm_token,
                path: h2.long_path_gm.clone(),
                ts: now_millis(),
                request_id: Uuid::nil(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        res2.stop,
        *h2.long_path_gm.last().unwrap(),
        "GM moves the full path — no truncation from an unresolvable per-cell scale"
    );
    assert_eq!(
        h2.gm_resource_current().await,
        10.0,
        "no decrement when the budget was unresolvable"
    );
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
