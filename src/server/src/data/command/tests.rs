use super::*;

fn doc(id: u128) -> Document {
    Document {
        id: Uuid::from_u128(id),
        scope: crate::data::document::Scope::World {
            world_id: Uuid::from_u128(9),
        },
        doc_type: "item".into(),
        schema_version: 1,
        name: None,
        source: None,
        base: None,
        owner: None,
        permissions: Default::default(),
        embedded: Default::default(),
        parent_id: None,
        engine: None,
        system: serde_json::json!({}),
        created_at: 0,
        updated_at: 0,
    }
}

#[test]
fn create_inverts_to_delete_and_back() {
    let op = Operation::Create { doc: doc(1) };
    assert_eq!(op.invert(), Operation::Delete { doc: doc(1) });
    assert_eq!(op.invert().invert(), op);
}

#[test]
fn update_invert_swaps_old_and_new_in_reverse() {
    let op = Operation::Update {
        doc_id: Uuid::from_u128(1),
        changes: vec![
            FieldChange {
                remove: false,
                path: "/system/a".into(),
                old: serde_json::json!(1),
                new: serde_json::json!(2),
            },
            FieldChange {
                remove: false,
                path: "/system/b".into(),
                old: serde_json::json!(3),
                new: serde_json::json!(4),
            },
        ],
    };
    let inv = op.invert();
    assert_eq!(
        inv,
        Operation::Update {
            doc_id: Uuid::from_u128(1),
            changes: vec![
                FieldChange {
                    remove: false,
                    path: "/system/b".into(),
                    old: serde_json::json!(4),
                    new: serde_json::json!(3)
                },
                FieldChange {
                    remove: false,
                    path: "/system/a".into(),
                    old: serde_json::json!(2),
                    new: serde_json::json!(1)
                },
            ],
        }
    );
    assert_eq!(op.invert().invert(), op);
}

#[test]
fn unsequenced_command_invert_is_round_trip() {
    let cmd = UnsequencedCommand {
        world_id: Uuid::from_u128(9),
        author: Uuid::from_u128(5),
        ts: 1,
        ops: vec![
            Operation::Create { doc: doc(1) },
            Operation::Update {
                doc_id: Uuid::from_u128(1),
                changes: vec![FieldChange {
                    remove: false,
                    path: "/system/x".into(),
                    old: serde_json::json!(null),
                    new: serde_json::json!(7),
                }],
            },
        ],
    };
    assert_eq!(cmd.invert().invert(), cmd);
}

#[test]
fn set_pointer_sets_existing_and_creates_intermediate() {
    let mut v = serde_json::json!({ "system": { "hp": 10 } });
    set_pointer(&mut v, "/system/hp", serde_json::json!(5)).unwrap();
    assert_eq!(v["system"]["hp"], serde_json::json!(5));

    set_pointer(&mut v, "/system/attributes/str", serde_json::json!(14)).unwrap();
    assert_eq!(v["system"]["attributes"]["str"], serde_json::json!(14));
}

#[test]
fn set_pointer_writes_into_an_indexed_embedded_actor_copy() {
    // An instanced token toggles conditions on its embedded actor copy at
    // `/embedded/actor/0/system/conditions`: array-index intermediate descent
    // followed by an object-leaf insert.
    let mut v =
        serde_json::json!({ "embedded": { "actor": [ { "system": { "conditions": [] } } ] } });
    set_pointer(
        &mut v,
        "/embedded/actor/0/system/conditions",
        serde_json::json!(["dead"]),
    )
    .unwrap();
    assert_eq!(
        v["embedded"]["actor"][0]["system"]["conditions"],
        serde_json::json!(["dead"])
    );
}

#[test]
fn set_pointer_descends_through_an_explicit_null_intermediate() {
    // `Option<T>` engine fields (e.g. SceneEngine.vision/lighting) serialize as
    // explicit `null`, not an absent key, so descending through one must succeed
    // exactly as it would through a missing key (never `BadPath`).
    let mut v = serde_json::json!({ "engine": { "vision": null } });
    set_pointer(
        &mut v,
        "/engine/vision/movementRestriction",
        serde_json::json!("revealed"),
    )
    .unwrap();
    assert_eq!(
        v,
        serde_json::json!({ "engine": { "vision": { "movementRestriction": "revealed" } } })
    );
}

#[test]
fn set_pointer_descends_through_two_nested_explicit_null_intermediates() {
    let mut v = serde_json::json!({ "engine": null });
    set_pointer(&mut v, "/engine/vision/mode", serde_json::json!("dark")).unwrap();
    assert_eq!(
        v,
        serde_json::json!({ "engine": { "vision": { "mode": "dark" } } })
    );
}

#[test]
fn set_pointer_rejects_descend_into_scalar() {
    let mut v = serde_json::json!({ "hp": 10 });
    let err = set_pointer(&mut v, "/hp/value", serde_json::json!(1));
    assert!(matches!(err, Err(DataError::BadPath(_))));
}

#[test]
fn remove_pointer_makes_an_object_key_genuinely_absent() {
    // A removed key is absent, NOT present-with-null (`null` != absent).
    let mut v = serde_json::json!({ "system": { "foo": "bar", "baz": 1 } });
    remove_pointer(&mut v, "/system/foo").unwrap();
    let sys = v["system"].as_object().unwrap();
    assert!(!sys.contains_key("foo"), "key must be absent, not null");
    assert_eq!(sys["baz"], serde_json::json!(1), "sibling keys untouched");
}

#[test]
fn remove_pointer_on_already_absent_key_is_a_no_op() {
    let mut v = serde_json::json!({ "system": { "baz": 1 } });
    remove_pointer(&mut v, "/system/foo").unwrap();
    assert_eq!(v, serde_json::json!({ "system": { "baz": 1 } }));
}

#[test]
fn remove_pointer_through_absent_intermediate_is_a_no_op() {
    // No intermediate is CREATED (unlike set_pointer): a target under a
    // missing ancestor is already absent, so removal is a silent success.
    let mut v = serde_json::json!({ "system": {} });
    remove_pointer(&mut v, "/system/missing/leaf").unwrap();
    assert_eq!(v, serde_json::json!({ "system": {} }));
}

#[test]
fn remove_pointer_through_a_null_intermediate_is_a_no_op() {
    // A `null` intermediate has nothing beneath it, so the target is already absent —
    // uniform with `set_pointer` and serde_json reads, which treat null == absent for descent.
    // The `null` itself is preserved (only the absent descendant "removal" is a no-op).
    let mut v = serde_json::json!({ "engine": { "vision": null } });
    remove_pointer(&mut v, "/engine/vision/mode").unwrap();
    assert_eq!(v, serde_json::json!({ "engine": { "vision": null } }));
}

#[test]
fn remove_pointer_rejects_array_index_removal() {
    // Array shrink is whole-array replacement only (merge-engine invariant):
    // a leaf remove of an index has no defined shift semantics.
    let mut v = serde_json::json!({ "tags": ["a", "b", "c"] });
    assert!(matches!(
        remove_pointer(&mut v, "/tags/1"),
        Err(DataError::BadPath(_))
    ));
    assert_eq!(v, serde_json::json!({ "tags": ["a", "b", "c"] }));
}

#[test]
fn remove_pointer_rejects_descend_into_scalar() {
    let mut v = serde_json::json!({ "hp": 10 });
    assert!(matches!(
        remove_pointer(&mut v, "/hp/value"),
        Err(DataError::BadPath(_))
    ));
}

#[test]
fn remove_pointer_rejects_missing_leading_slash_and_empty() {
    let mut v = serde_json::json!({ "system": { "hp": 10 } });
    assert!(matches!(
        remove_pointer(&mut v, "system/hp"),
        Err(DataError::BadPath(_))
    ));
    assert!(matches!(
        remove_pointer(&mut v, ""),
        Err(DataError::BadPath(_))
    ));
    assert_eq!(v, serde_json::json!({ "system": { "hp": 10 } }));
}

#[test]
fn remove_change_inverts_to_a_reinserting_set() {
    // Inverse of "remove key holding V" is "set key to V"; after the removal
    // the slot is absent, so the inverse's pre-image is Null.
    let op = Operation::Update {
        doc_id: Uuid::from_u128(1),
        changes: vec![FieldChange {
            path: "/system/foo".into(),
            old: serde_json::json!("bar"),
            new: serde_json::Value::Null,
            remove: true,
        }],
    };
    assert_eq!(
        op.invert(),
        Operation::Update {
            doc_id: Uuid::from_u128(1),
            changes: vec![FieldChange {
                path: "/system/foo".into(),
                old: serde_json::Value::Null,
                new: serde_json::json!("bar"),
                remove: false,
            }],
        }
    );
}

#[test]
fn set_pointer_rejects_missing_leading_slash() {
    // A pointer without a leading "/" must error, not silently write the
    // wrong field (e.g. "system/hp" must not land on top-level "hp").
    let mut v = serde_json::json!({ "system": { "hp": 10 } });
    assert!(matches!(
        set_pointer(&mut v, "system/hp", serde_json::json!(5)),
        Err(DataError::BadPath(_))
    ));
    assert!(matches!(
        set_pointer(&mut v, "foo", serde_json::json!(5)),
        Err(DataError::BadPath(_))
    ));
    assert_eq!(v, serde_json::json!({ "system": { "hp": 10 } }));
}

#[test]
fn command_round_trips_through_json() {
    use crate::data::document::{DocRole, PermissionSet, Scope, Source, Visibility};

    let mut perms = PermissionSet {
        default: DocRole::Observer,
        ..Default::default()
    };
    perms.users.insert(Uuid::from_u128(5), DocRole::Owner);
    perms
        .property_overrides
        .insert("/system/secret".into(), Visibility::GmOnly);

    let mut embedded = std::collections::BTreeMap::new();
    embedded.insert("items".to_string(), vec![doc(2)]);

    let rich = Document {
        id: Uuid::from_u128(1),
        scope: Scope::World {
            world_id: Uuid::from_u128(9),
        },
        doc_type: "actor".into(),
        schema_version: 1,
        name: None,
        source: Some(Source {
            id: Uuid::from_u128(3),
            pack: Some("dnd5e".into()),
            version: 2,
        }),
        base: None,
        owner: Some(Uuid::from_u128(5)),
        permissions: perms,
        embedded,
        parent_id: None,
        engine: None,
        system: serde_json::json!({ "hp": { "value": 10, "max": 12 }, "tags": ["a", "b"] }),
        created_at: 1,
        updated_at: 2,
    };

    let cmd = Command {
        seq: 7,
        world_id: Uuid::from_u128(9),
        author: Uuid::from_u128(5),
        ts: 100,
        ops: vec![
            Operation::Create { doc: rich },
            Operation::Delete { doc: doc(4) },
            Operation::Update {
                doc_id: Uuid::from_u128(1),
                changes: vec![
                    FieldChange {
                        remove: false,
                        path: "/system/hp/value".into(),
                        old: serde_json::json!(10),
                        new: serde_json::json!(3),
                    },
                    FieldChange {
                        remove: false,
                        path: "/name".into(),
                        old: serde_json::json!(null),
                        new: serde_json::json!("Gandalf"),
                    },
                ],
            },
        ],
    };

    let s = serde_json::to_string(&cmd).unwrap();
    assert!(
        s.contains("\"op\":\"create\""),
        "internally-tagged discriminator present"
    );
    let back: Command = serde_json::from_str(&s).unwrap();
    assert_eq!(cmd, back);
}

#[test]
fn move_inverts_by_swapping_parents() {
    let op = Operation::Move {
        doc_id: Uuid::from_u128(1),
        parent_id: Some(Uuid::from_u128(2)),
        old_parent_id: None,
    };
    assert_eq!(
        op.invert(),
        Operation::Move {
            doc_id: Uuid::from_u128(1),
            parent_id: None,
            old_parent_id: Some(Uuid::from_u128(2)),
        }
    );
    assert_eq!(op.invert().invert(), op);
}

#[test]
fn move_round_trips_through_json_with_op_tag() {
    let op = Operation::Move {
        doc_id: Uuid::from_u128(1),
        parent_id: Some(Uuid::from_u128(2)),
        old_parent_id: None,
    };
    let v = serde_json::to_value(&op).expect("serialize");
    assert_eq!(v["op"], serde_json::json!("move"));
    assert_eq!(
        v["doc_id"],
        serde_json::json!(Uuid::from_u128(1).to_string())
    );
    assert_eq!(
        v["parent_id"],
        serde_json::json!(Uuid::from_u128(2).to_string())
    );
    assert_eq!(v["old_parent_id"], serde_json::Value::Null);
    let back: Operation = serde_json::from_value(v).expect("deserialize");
    assert_eq!(back, op);
}
