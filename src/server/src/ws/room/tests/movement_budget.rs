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
    /// The world's `resource-registry` singleton (defines the `movement` binding).
    registry_id: Uuid,
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

    /// `token`'s combatant's current `movement` resource value, read off the live ECS through
    /// the same `SceneEcs` seam `Room::execute_move`'s gate uses. Read as the GM, whose
    /// `cap::READ` on every combatant is unconditional — this is a test observation of stored
    /// state, never a readability assertion.
    async fn resource_for(&self, token: Uuid) -> f64 {
        let world_defaults = self
            .repo
            .world_cap_defaults(self.world_id)
            .await
            .expect("world capability defaults");
        let scene = self.room.scene().read().await;
        let (combat_id, _) = scene
            .active_combat_for_scene(self.scene_id)
            .expect("active combat");
        let (_, ce, _) = scene
            .combatant_for_token(combat_id, token, &self.gm, &world_defaults)
            .expect("combatant for token");
        ce.resources["movement"].current
    }

    /// The player's combatant's current `movement` resource value.
    async fn resource_current(&self) -> f64 {
        self.resource_for(self.token_id).await
    }

    /// The GM combatant's current `movement` resource value.
    async fn gm_resource_current(&self) -> f64 {
        self.resource_for(self.gm_token).await
    }

    /// Severs every relationship the player has to their combatant DOCUMENT except the one
    /// `default`/`users` pair under test: `owner` is cleared, `permissions.default` is set to
    /// `default`, and `permissions.users` carries an entry for the player only when
    /// `user_entry` is `Some`. The player still owns the TOKEN, so they can still ask to move
    /// it — the exact reachability the hidden-combatant secrecy rule covers, and the shape
    /// `SceneEcs::combatant_for_token`'s `actor_id` fallback also produces for any token
    /// instanced from the same actor.
    async fn set_player_combatant_access(
        &self,
        default: crate::data::document::DocRole,
        user_entry: Option<crate::data::document::DocRole>,
    ) {
        let doc = self
            .repo
            .get_document(self.combatant_id)
            .await
            .unwrap()
            .expect("player combatant");
        let mut perms = doc.permissions.clone();
        perms.default = default;
        perms.users.clear();
        if let Some(role) = user_entry {
            perms.users.insert(self.player.user_id, role);
        }
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
                            new: serde_json::to_value(&perms).unwrap(),
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

    /// The combatant reads as unreadable to the player by `permissions.default` alone.
    async fn hide_player_combatant(&self) {
        self.set_player_combatant_access(crate::data::document::DocRole::None, None)
            .await;
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
                        old: serde_json::json!({ "current": 10.0 }),
                        new: serde_json::Value::Null,
                    }],
                }],
                0,
                WriteOrigin::Client,
            )
            .await
            .unwrap();
    }

    /// Swaps the registry's `movement` binding wholesale (a Mirror binding, a
    /// text `max`, …) — the knob for every "how does the gate resolve this
    /// binding" case.
    async fn set_registry_binding(&self, binding: serde_json::Value) {
        let doc = self
            .repo
            .get_document(self.registry_id)
            .await
            .unwrap()
            .expect("resource registry");
        let old = doc.engine.unwrap()["resources"]["movement"]["binding"].clone();
        self.room
            .publish(
                &self.repo,
                &self.gm,
                vec![Operation::Update {
                    doc_id: self.registry_id,
                    changes: vec![FieldChange {
                        remove: false,
                        path: "/engine/resources/movement/binding".into(),
                        old,
                        new: binding,
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
        "resources": { "movement": { "current": 10.0 } }
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
        "resources": { "movement": { "current": 10.0 } }
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

    // The registry defining the `movement` binding the gate resolves against:
    // Tracked, max 30, no recoveries.
    let registry_id = Uuid::from_u128(0x5CEA);
    let mut registry = wdoc(inner.world_id, registry_id, "resource-registry");
    registry.owner = Some(inner.gm.user_id);
    registry.engine = Some(json!({
        "resources": { "movement": { "name": "Movement", "order": 0,
            "binding": { "kind": "tracked", "max": 30.0,
                "recover": { "turn_start": 0, "turn_end": 0, "round_start": 0, "round_end": 0 } } } }
    }));
    inner
        .room
        .publish(
            &inner.repo,
            &inner.gm,
            vec![Operation::Create { doc: registry }],
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
        registry_id,
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

/// `BudgetGate::enforced` is whole-document `cap::READ`, not a `permissions.default` test, so a
/// per-user entry decides it in BOTH directions. A `default: none` combatant the mover holds an
/// explicit `users` grant on IS readable to them, so every gate protection applies — reading it
/// as unenforced would be an enforcement hole. Asserted against the same configuration
/// `not_the_turn_owner_is_rejected_under_hard_only` uses, so the difference under test is the
/// per-user grant and nothing else.
#[tokio::test]
async fn a_per_user_read_grant_on_a_default_hidden_combatant_still_enforces_the_gate() {
    use crate::data::document::DocRole;

    let h = budget_scene("hard", "spaces", None).await;
    h.set_player_combatant_access(DocRole::None, Some(DocRole::Observer))
        .await;
    h.set_turn(h.other_combatant).await;
    assert!(
        matches!(
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
        ),
        "a mover who can read the combatant is bound by its turn-owner rule, whatever \
         `permissions.default` says"
    );
    assert_eq!(
        h.committed_pos(h.token_id).await,
        h.start,
        "the token must not have moved"
    );
}

/// The other direction of the same rule: a `default: observer` combatant carrying a per-user
/// `none` override for THIS mover is unreadable to them, so neither the turn-owner refusal nor
/// the truncation may apply — either would disclose the combatant's existence or its exact
/// numeric budget to someone who never receives that document at egress. Same shape as
/// `a_hidden_combatant_neither_refuses_nor_truncates_a_mover_who_cannot_read_it`, reached
/// through the per-user override instead of the default.
#[tokio::test]
async fn a_per_user_override_makes_an_otherwise_readable_combatant_unenforced() {
    use crate::data::document::DocRole;

    let h = budget_scene("hard", "spaces", None).await;
    h.set_player_combatant_access(DocRole::Observer, Some(DocRole::None))
        .await;
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
        "a refusal here would disclose a combatant this mover cannot read"
    );

    let h = budget_scene("hard", "spaces", None).await;
    h.set_player_combatant_access(DocRole::Observer, Some(DocRole::None))
        .await;
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
        "truncation would disclose the budget of a combatant this mover cannot read"
    );
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
    // The registry's `movement` binding is Mirror-bound — unresolvable for the
    // gate (a spend cannot decrement a derived value; the server never writes
    // the system band). For a GM this must degrade to "move freely, no
    // decrement".
    let h = budget_scene("hard", "spaces", None).await;
    h.set_registry_binding(serde_json::json!({ "kind": "mirror", "value": "hp" }))
        .await;
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
    assert_eq!(
        h.gm_resource_current().await,
        10.0,
        "no decrement when the binding cannot resolve to a spendable value"
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
async fn missing_per_cell_scale_is_refused_with_the_generic_error() {
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
}

#[tokio::test]
async fn an_absent_resource_entry_reads_as_a_full_budget_and_materializes_on_spend() {
    let h = budget_scene("hard", "spaces", None).await;
    h.remove_resource_entry().await;
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
        "an untouched entry reads as the evaluated max (30), not a refusal — 15 cells fit"
    );
    assert_eq!(
        h.resource_current().await,
        15.0,
        "the spend materialized the entry at full-minus-cost"
    );
}

#[tokio::test]
async fn a_text_max_evaluated_over_the_linked_actor_gates_and_clamps_the_budget() {
    let h = budget_scene("hard", "spaces", None).await;
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let actor_id = Uuid::from_u128(0x5CEB);
    let mut actor = wdoc(h.world_id, actor_id, "actor");
    actor.owner = Some(h.player.user_id);
    actor.engine = Some(serde_json::json!({
        "displayName": "Runner", "visual": { "kind": "image", "asset": "a.png" },
        "size": { "w": 1.0, "h": 1.0 }, "shape": "square", "conditions": [],
        "prototype": false, "vision": null,
    }));
    actor.system = serde_json::json!({ "spd": 2.0 });
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Create { doc: actor }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Update {
                doc_id: h.combatant_id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/engine/kind/actor_id".into(),
                    old: serde_json::Value::Null,
                    new: serde_json::json!(actor_id),
                }],
            }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
    h.set_registry_binding(serde_json::json!({ "kind": "tracked", "max": "spd",
        "recover": { "turn_start": 0, "turn_end": 0, "round_start": 0, "round_end": 0 } }))
        .await;
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
    assert_eq!(
        res.stop, h.adj2,
        "the stored current (10) clamps to the evaluated text max (2) and gates there"
    );
    assert_eq!(h.resource_current().await, 0.0, "spent to the floor");
}

#[tokio::test]
async fn a_mirror_binding_refuses_the_enforced_mover_and_an_eval_error_does_too() {
    let h = budget_scene("hard", "spaces", None).await;
    h.set_registry_binding(serde_json::json!({ "kind": "mirror", "value": "hp" }))
        .await;
    assert!(
        matches!(
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
        ),
        "a Mirror-bound movement resource is unresolvable for an enforced mover"
    );

    let h = budget_scene("hard", "spaces", None).await;
    // No formula host anywhere (the combatant links no actor and the token
    // embeds no copy), so a referencing text max cannot resolve.
    h.set_registry_binding(serde_json::json!({ "kind": "tracked", "max": "spd",
        "recover": { "turn_start": 0, "turn_end": 0, "round_start": 0, "round_end": 0 } }))
        .await;
    assert!(
        matches!(
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
        ),
        "an evaluation error is unresolvable for an enforced mover"
    );
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

/// Links the budget scene's player token to a fresh world actor carrying `movement` tags (the
/// terrain-exemption source `token_movement_tags` resolves), through two ordinary publishes:
/// the actor create, then a wholesale `/engine` write whose OCC pre-image is the token's LIVE
/// stored engine body (read back from the repo, never re-derivable fixture text).
async fn link_movement_actor(h: &BudgetHandle, movement: serde_json::Value) {
    use serde_json::json;
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let actor_id = Uuid::from_u128(0x5CEB);
    let mut actor = wdoc(h.world_id, actor_id, "actor");
    actor.owner = Some(h.gm.user_id);
    actor.engine = Some(json!({
        "displayName": "Skyborn",
        "visual": { "kind": "image", "asset": "a" },
        "size": { "w": 1.0, "h": 1.0 },
        "shape": "square",
        "faction": null,
        "conditions": [],
        "prototype": false,
        "movement": movement
    }));
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Create { doc: actor }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();

    let stored = h
        .repo
        .get_document(h.token_id)
        .await
        .unwrap()
        .expect("player token")
        .engine
        .expect("engine body");
    let mut linked = stored.clone();
    linked["actor_id"] = json!(actor_id.to_string());
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Update {
                doc_id: h.token_id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/engine".into(),
                    old: stored,
                    new: linked,
                }],
            }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
}

/// Publishes a ×3 terrain region over the three cells the `start → adj → adj2 → adj3` walk
/// enters, parented to the budget scene.
async fn add_terrain_band(h: &BudgetHandle) {
    use serde_json::json;
    let wdoc = crate::data::document::tests::world_scoped_doc;
    let mut region = wdoc(h.world_id, Uuid::from_u128(0x5CEC), "region");
    region.parent_id = Some(h.scene_id);
    region.owner = Some(h.gm.user_id);
    region.engine = Some(json!({
        "shape": { "kind": "rect", "points": [100.0, 0.0, 400.0, 100.0] },
        "behavior": "terrain",
        "cost": 3.0,
        "enabled": true
    }));
    h.room
        .publish(
            &h.repo,
            &h.gm,
            vec![Operation::Create { doc: region }],
            0,
            WriteOrigin::Client,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn exempt_mover_decrements_the_exempt_cost_where_the_ground_mover_pays_the_multiplier() {
    use serde_json::json;
    // Budget 10 cells (current 10 / perCell 1); the two-step walk crosses two ×3 cells, so the
    // ground mover pays 6 and the flying mover pays 2 — the SAME move, the SAME budget gate,
    // and the decrement consumes the executor's exempt `MoveOutcome.cost` unchanged.
    let ground = budget_scene("hard", "per_cell", Some(1.0)).await;
    add_terrain_band(&ground).await;
    let flying = budget_scene("hard", "per_cell", Some(1.0)).await;
    add_terrain_band(&flying).await;
    link_movement_actor(&flying, json!(["flying"])).await;

    let ground_res = ground
        .room
        .execute_move(
            &ground.repo,
            &ground.player,
            crate::ws::room::MoveRequestInputs {
                scene_id: ground.scene_id,
                token: ground.token_id,
                path: vec![ground.start, ground.adj, ground.adj2],
                ts: now_millis(),
                request_id: Uuid::nil(),
            },
        )
        .await
        .unwrap();
    assert_eq!(ground_res.stop, ground.adj2);
    assert_eq!(
        ground.resource_current().await,
        4.0,
        "ground mover: 10 - 2 steps × 3"
    );

    let fly_res = flying
        .room
        .execute_move(
            &flying.repo,
            &flying.player,
            crate::ws::room::MoveRequestInputs {
                scene_id: flying.scene_id,
                token: flying.token_id,
                path: vec![flying.start, flying.adj, flying.adj2],
                ts: now_millis(),
                request_id: Uuid::nil(),
            },
        )
        .await
        .unwrap();
    assert_eq!(fly_res.stop, flying.adj2);
    assert_eq!(
        flying.resource_current().await,
        8.0,
        "flying mover: 10 - 2 steps × 1 (terrain multiplier reads as 1.0)"
    );
}
