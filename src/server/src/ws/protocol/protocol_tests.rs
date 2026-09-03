use super::*;
use uuid::Uuid;

#[test]
fn asset_changed_is_out_of_band_and_serializes_snake_case() {
    let m = ServerMsg::AssetChanged {
        uuid: Uuid::from_u128(7),
        op: AssetOp::Replaced,
        version: 3,
    };
    // Out-of-band: no event seq, so egress sends it without gap/resync logic.
    assert_eq!(m.event_seq(), None);
    let s = serde_json::to_string(&m).unwrap();
    assert!(s.contains("\"type\":\"asset_changed\""), "got {s}");
    assert!(s.contains("\"op\":\"replaced\""), "got {s}");
    assert!(s.contains("\"version\":3"), "got {s}");

    // Deleted carries the pre-removal version, a real ordering token.
    let d = ServerMsg::AssetChanged {
        uuid: Uuid::from_u128(7),
        op: AssetOp::Deleted,
        version: 5,
    };
    let s = serde_json::to_string(&d).unwrap();
    assert!(s.contains("\"op\":\"deleted\""), "got {s}");
    assert!(s.contains("\"version\":5"), "got {s}");
}

#[test]
fn scene_ping_round_trips_and_is_out_of_band() {
    let c = ClientMsg::ScenePing {
        scene: Uuid::from_u128(1),
        x: 10.0,
        y: 20.0,
    };
    let s = serde_json::to_string(&c).unwrap();
    assert!(s.contains("\"type\":\"scene_ping\""), "got {s}");
    let _back: ClientMsg = serde_json::from_str(&s).unwrap();

    let sv = ServerMsg::ScenePing {
        scene: Uuid::from_u128(1),
        x: 10.0,
        y: 20.0,
        user: Uuid::from_u128(2),
    };
    // Out-of-band: never buffered/resynced.
    assert_eq!(sv.event_seq(), None);
    let j = serde_json::to_value(&sv).unwrap();
    assert_eq!(j["type"], "scene_ping");
    assert_eq!(j["x"], 10.0);
    assert!(j.get("user").is_some());
}

#[test]
fn emote_round_trips_and_is_out_of_band() {
    let c = ClientMsg::Emote {
        scene: Uuid::from_u128(1),
        token: Uuid::from_u128(2),
        emote: "😀".to_string(),
    };
    let s = serde_json::to_string(&c).unwrap();
    assert!(s.contains("\"type\":\"emote\""), "got {s}");
    let _back: ClientMsg = serde_json::from_str(&s).unwrap();

    let sv = ServerMsg::Emote {
        scene: Uuid::from_u128(1),
        token: Uuid::from_u128(2),
        user: Uuid::from_u128(3),
        emote: "😀".to_string(),
    };
    // Out-of-band: never buffered/resynced.
    assert_eq!(sv.event_seq(), None);
    let j = serde_json::to_value(&sv).unwrap();
    assert_eq!(j["type"], "emote");
    assert_eq!(
        j["token"],
        serde_json::to_value(Uuid::from_u128(2)).unwrap()
    );
    assert_eq!(j["emote"], "😀");
    assert!(j.get("user").is_some());
}

#[test]
fn client_hello_round_trips_and_is_tagged() {
    let m = ClientMsg::Hello {
        world: Uuid::from_u128(7),
        last_seq: Some(3),
    };
    let s = serde_json::to_string(&m).unwrap();
    assert!(s.contains("\"type\":\"hello\""));
    let back: ClientMsg = serde_json::from_str(&s).unwrap();
    assert!(matches!(
        back,
        ClientMsg::Hello {
            last_seq: Some(3),
            ..
        }
    ));
}

#[test]
fn server_event_and_resync_round_trip() {
    let begin = ServerMsg::ResyncBegin {
        from_seq: 2,
        to_seq: 5,
        source: ResyncSource::Buffer,
    };
    let s = serde_json::to_string(&begin).unwrap();
    assert!(s.contains("\"type\":\"resync_begin\""));
    assert!(s.contains("\"source\":\"buffer\""));
    let _back: ServerMsg = serde_json::from_str(&s).unwrap();
}

#[test]
fn reject_round_trips_snake_case() {
    let m = ServerMsg::Reject {
        intent_id: Uuid::from_u128(3),
        reason: RejectReason::Conflict,
    };
    let s = serde_json::to_string(&m).unwrap();
    assert!(s.contains("\"type\":\"reject\""));
    assert!(s.contains("\"reason\":\"conflict\""));
    let _back: ServerMsg = serde_json::from_str(&s).unwrap();
}

#[test]
fn error_code_serializes_snake_case() {
    let e = ServerMsg::Error {
        code: WsErrorCode::WorldNotFound,
        message: "x".into(),
    };
    let s = serde_json::to_string(&e).unwrap();
    assert!(s.contains("\"code\":\"world_not_found\""));
}

#[test]
fn search_frames_round_trip() {
    let req = ClientMsg::Search {
        request_id: Uuid::from_u128(1),
        query: "dragon".into(),
        limit: 20,
        cursor: None,
        subscribe: false,
    };
    let s = serde_json::to_string(&req).unwrap();
    assert!(s.contains("\"type\":\"search\""));
    let _back: ClientMsg = serde_json::from_str(&s).unwrap();

    let err = ServerMsg::SearchError {
        request_id: Uuid::from_u128(2),
        message: "x".into(),
    };
    let s = serde_json::to_string(&err).unwrap();
    assert!(s.contains("\"type\":\"search_error\""));
}

#[test]
fn subscribe_defaults_false_and_live_frames_round_trip() {
    // A one-shot Search frame (no `subscribe`) still deserializes (default false).
    let oneshot: ClientMsg = serde_json::from_str(
        r#"{"type":"search","request_id":"00000000-0000-0000-0000-000000000001","query":"x","limit":20,"cursor":null}"#,
    )
    .unwrap();
    match oneshot {
        ClientMsg::Search { subscribe, .. } => assert!(!subscribe),
        _ => panic!("expected Search"),
    }
    let unsub = ClientMsg::Unsubscribe {
        request_id: Uuid::from_u128(1),
    };
    assert!(serde_json::to_string(&unsub)
        .unwrap()
        .contains("\"type\":\"unsubscribe\""));
    let upd = ServerMsg::SearchUpdate {
        request_id: Uuid::from_u128(2),
        hits: Vec::new(),
    };
    assert!(serde_json::to_string(&upd)
        .unwrap()
        .contains("\"type\":\"search_update\""));
}

#[test]
fn pathfind_frames_round_trip() {
    let req = ClientMsg::Pathfind {
        request_id: Uuid::from_u128(1),
        scene: Uuid::from_u128(2),
        start: (50.0, 50.0),
        waypoints: vec![(150.0, 50.0), (250.0, 50.0)],
        footprint_radius: 0.5,
        token: None,
    };
    let s = serde_json::to_string(&req).unwrap();
    assert!(s.contains("\"type\":\"pathfind\""), "got {s}");
    let back: ClientMsg = serde_json::from_str(&s).unwrap();
    assert!(matches!(back, ClientMsg::Pathfind { .. }));

    let ok = ServerMsg::PathResult {
        request_id: Uuid::from_u128(1),
        path: vec![(50.0, 50.0)],
        cost: 2.0,
        arrested: true,
        truncated: false,
        budget_cells: Some(6.0),
    };
    let json = serde_json::to_string(&ok).unwrap();
    assert!(json.contains("\"type\":\"path_result\""));
    let back: ServerMsg = serde_json::from_str(&json).unwrap();
    match back {
        ServerMsg::PathResult {
            arrested,
            budget_cells,
            ..
        } => {
            assert!(arrested);
            assert_eq!(budget_cells, Some(6.0));
        }
        _ => panic!("expected PathResult"),
    }
    let err = ServerMsg::PathError {
        request_id: Uuid::from_u128(1),
        message: "unreachable".into(),
    };
    assert!(serde_json::to_string(&err)
        .unwrap()
        .contains("\"type\":\"path_error\""));
}

#[test]
fn scene_frames_round_trip() {
    let sub = ClientMsg::SceneSubscribe {
        request_id: Uuid::from_u128(1),
        channel: "identity".into(),
        as_user: None,
    };
    let j = serde_json::to_value(&sub).unwrap();
    assert_eq!(j["type"], "scene_subscribe");
    assert_eq!(j["channel"], "identity");

    let d = ServerMsg::SceneDerived {
        request_id: Uuid::from_u128(1),
        channel: "identity".into(),
        computed_at_seq: 7,
        payload: serde_json::json!({ "entity_count": 3 }),
    };
    let j = serde_json::to_value(&d).unwrap();
    assert_eq!(j["type"], "scene_derived");
    assert_eq!(j["computed_at_seq"], 7);
    assert_eq!(j["payload"]["entity_count"], 3);
}

#[test]
fn move_request_round_trip() {
    let req = ClientMsg::MoveRequest {
        request_id: Uuid::from_u128(1),
        scene: Uuid::from_u128(2),
        token_id: Uuid::from_u128(3),
        path: vec![[0.0, 0.0], [100.0, 0.0], [100.0, 100.0]],
    };
    let wire = serde_json::to_string(&req).unwrap();
    assert!(wire.contains("\"type\":\"move_request\""), "got {wire}");
    let back: ClientMsg = serde_json::from_str(&wire).unwrap();
    assert!(matches!(back, ClientMsg::MoveRequest { .. }));

    // Server replies with MoveError (rejection path) or MoveStream (success path); no MoveExecuted.
    let err = ServerMsg::MoveError {
        request_id: Uuid::from_u128(1),
        message: "token is moving".into(),
    };
    assert!(serde_json::to_string(&err).unwrap().contains("move_error"));
}

#[test]
fn move_stream_round_trips_and_is_tagged() {
    let in_samples = vec![PosSample {
        t_ms: 0.0,
        pos: [0.0, 0.0],
    }];
    let in_vision = Some(vec![VisionSample {
        t_ms: 0.0,
        polygons: vec![vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]],
    }]);
    let in_light = Some(vec![LightSample {
        t_ms: 0.0,
        pos: [0.0, 0.0],
        bright: 200.0,
        dim: 600.0,
        intensity: 0.8,
        falloff: crate::data::engine::FalloffCurve::Quadratic,
        color: 0xFFD9A0,
        polygons: vec![vec![[-600.0, -600.0], [600.0, -600.0], [600.0, 600.0]]],
    }]);
    let msg = ServerMsg::MoveStream {
        request_id: Uuid::from_u128(1),
        token_id: Uuid::from_u128(2),
        mover: Uuid::from_u128(3),
        scene: Uuid::from_u128(4),
        start_server_ms: 1000.0,
        duration_ms: 500.0,
        stop: [100.0, 200.0],
        samples: in_samples.clone(),
        mover_vision: in_vision.clone(),
        mover_light: in_light.clone(),
        cost: Some(3.5),
        truncated: Some(true),
    };
    let wire = serde_json::to_string(&msg).unwrap();
    // Tag must be snake_case.
    assert!(wire.contains("\"type\":\"move_stream\""), "got {wire}");
    // Deserializes back; each field survives the round-trip.
    let back: ServerMsg = serde_json::from_str(&wire).unwrap();
    match back {
        ServerMsg::MoveStream {
            request_id,
            token_id,
            mover,
            scene,
            start_server_ms,
            duration_ms,
            stop,
            samples,
            mover_vision,
            mover_light,
            cost,
            truncated,
        } => {
            assert_eq!(request_id, Uuid::from_u128(1));
            assert_eq!(token_id, Uuid::from_u128(2));
            assert_eq!(mover, Uuid::from_u128(3));
            assert_eq!(scene, Uuid::from_u128(4));
            assert_eq!(start_server_ms, 1000.0);
            assert_eq!(duration_ms, 500.0);
            assert_eq!(stop, [100.0, 200.0]);
            assert_eq!(samples, in_samples);
            assert_eq!(mover_vision, in_vision);
            assert_eq!(
                mover_light, in_light,
                "an admitted carried-light timeline survives the round-trip field for field"
            );
            assert_eq!(cost, Some(3.5), "mover/GM path: cost is disclosed");
            assert_eq!(
                truncated,
                Some(true),
                "mover/GM path: truncation is disclosed"
            );
        }
        _ => panic!("expected MoveStream"),
    }
    // None mover_vision + None cost path — verify both round-trip as None for a clipped observer.
    let in_samples2 = vec![PosSample {
        t_ms: 0.0,
        pos: [0.0, 0.0],
    }];
    let msg_no_vision = ServerMsg::MoveStream {
        request_id: Uuid::from_u128(1),
        token_id: Uuid::from_u128(2),
        mover: Uuid::from_u128(3),
        scene: Uuid::from_u128(4),
        start_server_ms: 1000.0,
        duration_ms: 500.0,
        stop: [100.0, 200.0],
        samples: in_samples2,
        mover_vision: None,
        mover_light: None,
        cost: None,
        truncated: None,
    };
    let wire2 = serde_json::to_string(&msg_no_vision).unwrap();
    let back2: ServerMsg = serde_json::from_str(&wire2).unwrap();
    match back2 {
        ServerMsg::MoveStream {
            mover_vision,
            mover_light,
            cost,
            truncated,
            ..
        } => {
            assert_eq!(
                mover_vision, None,
                "observer path: mover_vision must round-trip as None"
            );
            assert_eq!(
                mover_light, None,
                "a recipient no light sample reaches gets no timeline at all, never an empty one"
            );
            assert_eq!(
                cost, None,
                "observer path: cost must round-trip as None (secrecy: must not disclose \
                 authoritative cost, which may reflect secret terrain, to an observer)"
            );
            assert_eq!(
                truncated, None,
                "observer path: truncated must round-trip as None (secrecy: a truthful flag \
                 answers whether anything stopped the token BEYOND the observer's clipped \
                 view, disclosing a wall or gm_only region they cannot see)"
            );
        }
        _ => panic!("expected MoveStream"),
    }
}

#[test]
fn send_message_frame_parses() {
    let raw = r#"{"type":"send_message","request_id":"00000000-0000-0000-0000-0000000000aa","channel":"all","content":"hi","actor_owner":null}"#;
    let msg: ClientMsg = serde_json::from_str(raw).unwrap();
    match msg {
        ClientMsg::SendMessage {
            request_id,
            channel,
            content,
            actor_owner,
            audience,
        } => {
            assert_eq!(request_id, Uuid::from_u128(0xaa));
            assert_eq!(channel, "all");
            assert_eq!(content, "hi");
            assert!(actor_owner.is_none());
            assert_eq!(
                audience,
                crate::chat::Audience::Public,
                "omitted audience defaults to Public"
            );
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn edit_and_delete_frames_carry_request_id() {
    let edit: ClientMsg = serde_json::from_str(
        r#"{"type":"edit_message","request_id":"00000000-0000-0000-0000-0000000000ab","message_id":"00000000-0000-0000-0000-000000000001","content":"fixed"}"#,
    )
    .unwrap();
    match edit {
        ClientMsg::EditMessage { request_id, .. } => {
            assert_eq!(request_id, Uuid::from_u128(0xab))
        }
        _ => panic!("wrong variant"),
    }
    let del: ClientMsg = serde_json::from_str(
        r#"{"type":"delete_message","request_id":"00000000-0000-0000-0000-0000000000ac","message_id":"00000000-0000-0000-0000-000000000001"}"#,
    )
    .unwrap();
    match del {
        ClientMsg::DeleteMessage { request_id, .. } => {
            assert_eq!(request_id, Uuid::from_u128(0xac))
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn recalc_roll_frame_parses() {
    let raw = r#"{"type":"recalc_roll","request_id":"00000000-0000-0000-0000-0000000000ad","message_id":"00000000-0000-0000-0000-000000000001","roll_id":"00000000-0000-0000-0000-000000000002","ops":[{"kind":"reroll_dice","ids":[1,2]},{"kind":"replace_die","id":3,"natural":6},{"kind":"remove_dice","ids":[4]}]}"#;
    let msg: ClientMsg = serde_json::from_str(raw).unwrap();
    match msg {
        ClientMsg::RecalcRoll {
            request_id,
            message_id,
            roll_id,
            ops,
        } => {
            assert_eq!(request_id, Uuid::from_u128(0xad));
            assert_eq!(message_id, Uuid::from_u128(1));
            assert_eq!(roll_id, Uuid::from_u128(2));
            assert_eq!(ops.len(), 3);
            assert!(matches!(
                ops[0],
                crate::chat::WireRecalcOp::RerollDice { .. }
            ));
            assert!(matches!(
                ops[1],
                crate::chat::WireRecalcOp::ReplaceDie { id: 3, natural: 6 }
            ));
            assert!(matches!(
                ops[2],
                crate::chat::WireRecalcOp::RemoveDice { .. }
            ));
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn chat_error_frame_round_trips_and_is_tagged() {
    let e = ServerMsg::ChatError {
        request_id: Uuid::from_u128(9),
        message: "You are not permitted to send this message.".into(),
    };
    // Correlated reply, not a sequenced Event: never buffered/resynced.
    assert_eq!(e.event_seq(), None);
    let s = serde_json::to_string(&e).unwrap();
    assert!(s.contains("\"type\":\"chat_error\""), "got {s}");
    let _back: ServerMsg = serde_json::from_str(&s).unwrap();
}

#[test]
fn send_message_frame_parses_whisper_audience() {
    let raw = r#"{"type":"send_message","request_id":"00000000-0000-0000-0000-0000000000aa","channel":"all","content":"psst","actor_owner":null,"audience":{"kind":"whisper","recipients":["00000000-0000-0000-0000-000000000001"]}}"#;
    let msg: ClientMsg = serde_json::from_str(raw).unwrap();
    match msg {
        ClientMsg::SendMessage { audience, .. } => {
            assert_eq!(
                audience,
                crate::chat::Audience::Whisper {
                    recipients: vec![Uuid::from_u128(1)]
                }
            );
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn send_message_frame_parses_gm_only_audience() {
    let raw = r#"{"type":"send_message","request_id":"00000000-0000-0000-0000-0000000000aa","channel":"gm","content":"for your eyes only","actor_owner":null,"audience":{"kind":"gm_only"}}"#;
    let msg: ClientMsg = serde_json::from_str(raw).unwrap();
    match msg {
        ClientMsg::SendMessage { audience, .. } => {
            assert_eq!(audience, crate::chat::Audience::GmOnly);
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn combat_intents_round_trip_snake_case_tags() {
    let v = serde_json::json!({ "type": "combat_roll", "request_id": Uuid::nil(), "combat_id": Uuid::nil(), "channel": "table",
        "rolls": [{ "combatant_id": Uuid::nil(), "notation": "1d20+3" }] });
    let m: ClientMsg = serde_json::from_value(v).unwrap();
    assert!(matches!(m, ClientMsg::CombatRoll { ref rolls, .. } if rolls.len() == 1));
    let v = serde_json::json!({ "type": "combat_resource", "request_id": Uuid::nil(), "combat_id": Uuid::nil(), "combatant_id": Uuid::nil(),
        "resource": "movement", "op": { "kind": "delta", "amount": -5.0 } });
    assert!(
        matches!(serde_json::from_value::<ClientMsg>(v).unwrap(), ClientMsg::CombatResource { op: ResourceOp::Delta { amount }, .. } if amount == -5.0)
    );
    let e = ServerMsg::CombatError {
        request_id: Uuid::nil(),
        message: "combat rejected".into(),
    };
    assert_eq!(serde_json::to_value(&e).unwrap()["type"], "combat_error");

    let r = ServerMsg::CombatResult {
        request_id: Uuid::nil(),
        seq: 7,
    };
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(v["type"], "combat_result");
    assert_eq!(v["seq"], 7);
}

#[test]
fn combat_start_pause_end_advance_rewind_sort_round_trip() {
    for (variant, tag) in [
        (
            ClientMsg::CombatStart {
                request_id: Uuid::from_u128(1),
                combat_id: Uuid::from_u128(2),
            },
            "combat_start",
        ),
        (
            ClientMsg::CombatPause {
                request_id: Uuid::from_u128(1),
                combat_id: Uuid::from_u128(2),
            },
            "combat_pause",
        ),
        (
            ClientMsg::CombatEnd {
                request_id: Uuid::from_u128(1),
                combat_id: Uuid::from_u128(2),
            },
            "combat_end",
        ),
        (
            ClientMsg::CombatAdvance {
                request_id: Uuid::from_u128(1),
                combat_id: Uuid::from_u128(2),
            },
            "combat_advance",
        ),
        (
            ClientMsg::CombatRewind {
                request_id: Uuid::from_u128(1),
                combat_id: Uuid::from_u128(2),
            },
            "combat_rewind",
        ),
        (
            ClientMsg::CombatSort {
                request_id: Uuid::from_u128(1),
                combat_id: Uuid::from_u128(2),
            },
            "combat_sort",
        ),
    ] {
        let s = serde_json::to_string(&variant).unwrap();
        assert!(
            s.contains(&format!("\"type\":\"{tag}\"")),
            "got {s}, wanted tag {tag}"
        );
        let back: ClientMsg = serde_json::from_str(&s).unwrap();
        assert_eq!(
            serde_json::to_value(&back).unwrap()["type"],
            serde_json::to_value(&variant).unwrap()["type"]
        );
    }
}

#[test]
fn combat_resource_set_op_round_trips() {
    let m = ClientMsg::CombatResource {
        request_id: Uuid::from_u128(1),
        combat_id: Uuid::from_u128(2),
        combatant_id: Uuid::from_u128(3),
        resource: "movement".into(),
        op: ResourceOp::Set { value: 10.0 },
    };
    let s = serde_json::to_string(&m).unwrap();
    assert!(s.contains("\"kind\":\"set\""), "got {s}");
    let back: ClientMsg = serde_json::from_str(&s).unwrap();
    assert!(matches!(
        back,
        ClientMsg::CombatResource {
            op: ResourceOp::Set { value },
            ..
        } if value == 10.0
    ));
}

#[test]
fn welcome_carries_caps_role_and_requirements() {
    use crate::data::document::{CapabilityGrants, WorldRole};
    let w = ServerMsg::Welcome {
        world: Uuid::from_u128(1),
        current_seq: 0,
        server_time: 0,
        server_version: "0.0.0-test".to_string(),
        world_default_grants: CapabilityGrants::default(),
        user_role: WorldRole::Player,
        capability_requirements: Vec::new(),
        contract_declarations: Vec::new(),
        schema_declarations: Vec::new(),
    };
    let json = serde_json::to_value(&w).unwrap();
    assert_eq!(json["type"], "welcome");
    assert_eq!(json["user_role"], "player");
    assert!(json.get("world_default_grants").is_some());
    assert!(json.get("capability_requirements").is_some());
    assert!(json.get("contract_declarations").is_some());
    assert!(json.get("schema_declarations").is_some());
    assert_eq!(json["server_version"], "0.0.0-test");
}
