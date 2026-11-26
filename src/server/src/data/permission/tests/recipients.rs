use super::*;
use crate::data::snapshot::{CommandSnapshot, OpSnapshot};
use crate::test_ids::test_uuid;

/// An `Access` for `user_id` holding no caps and no tier flags.
fn viewer(user_id: Option<uuid::Uuid>) -> Access {
    Access {
        caps: Default::default(),
        all: false,
        see_gm_only: false,
        is_owner: false,
        user_id,
    }
}

/// The wire form of every tier round-trips: bare strings for the role tiers,
/// `{"recipients": [...]}` for a list. Fails if the untagged `Recipients`
/// variant stops deserializing or shadows a bare-string tier.
#[test]
fn visibility_wire_form_round_trips() {
    let alice = test_uuid(0xA1);
    let list = Visibility::Recipients {
        recipients: vec![alice],
    };
    let json = serde_json::to_value(&list).unwrap();
    assert_eq!(
        json,
        serde_json::json!({ "recipients": [alice.to_string()] })
    );
    assert_eq!(serde_json::from_value::<Visibility>(json).unwrap(), list);
    for tier in [Visibility::All, Visibility::GmOnly, Visibility::OwnerOrGm] {
        let json = serde_json::to_value(&tier).unwrap();
        assert!(json.is_string(), "role tiers stay bare strings");
        assert_eq!(serde_json::from_value::<Visibility>(json).unwrap(), tier);
    }
}

#[test]
fn recipients_tier_is_exact_listed_membership() {
    let alice = test_uuid(0xA1);
    let bob = test_uuid(0xB0);
    let tier = Visibility::Recipients {
        recipients: vec![alice],
    };
    assert!(viewer(Some(alice)).can_see(&tier), "listed user sees it");
    assert!(!viewer(Some(bob)).can_see(&tier), "unlisted user is denied");
    assert!(
        !viewer(None).can_see(&tier),
        "the viewer-less public-index Access never matches a list"
    );
}

#[test]
fn recipients_tier_never_grants_by_role() {
    let alice = test_uuid(0xA1);
    let gm_id = test_uuid(0x6D);
    let tier = Visibility::Recipients {
        recipients: vec![alice],
    };
    let unlisted_gm = Access {
        all: true,
        see_gm_only: true,
        is_owner: true,
        ..viewer(Some(gm_id))
    };
    assert!(
        !unlisted_gm.can_see(&tier),
        "a GM that is not individually listed sees nothing"
    );
    let listed_gm = Visibility::Recipients {
        recipients: vec![gm_id],
    };
    assert!(unlisted_gm.can_see(&listed_gm), "a listed GM sees it");
    assert!(
        !viewer(Some(alice)).can_see(&Visibility::Recipients {
            recipients: Vec::new()
        }),
        "an empty list admits nobody"
    );
}

#[test]
fn the_three_pre_existing_tiers_are_unchanged_by_reference_evaluation() {
    let gm = Access {
        see_gm_only: true,
        ..viewer(None)
    };
    let owner = Access {
        is_owner: true,
        ..viewer(None)
    };
    let nobody = viewer(None);
    for (access, all, gm_only, owner_or_gm) in [
        (&gm, true, true, true),
        (&owner, true, false, true),
        (&nobody, true, false, false),
    ] {
        assert_eq!(access.can_see(&Visibility::All), all);
        assert_eq!(access.can_see(&Visibility::GmOnly), gm_only);
        assert_eq!(access.can_see(&Visibility::OwnerOrGm), owner_or_gm);
    }
}

#[test]
fn filter_properties_strips_a_recipients_tiered_property_for_unlisted_viewers() {
    let alice = test_uuid(0xA1);
    let bob = test_uuid(0xB0);
    let d = doc(
        perms_with(&[(
            "/system/secret",
            Visibility::Recipients {
                recipients: vec![alice],
            },
        )]),
        serde_json::json!({ "secret": "MOCK_SECRET_A", "open": 1 }),
    );

    let for_alice = filter_properties(&d, &viewer(Some(alice))).unwrap();
    assert_eq!(for_alice.system["secret"], "MOCK_SECRET_A");

    for unlisted in [viewer(Some(bob)), viewer(None)] {
        let seen = filter_properties(&d, &unlisted).unwrap();
        assert!(
            seen.system.get("secret").is_none_or(|v| v.is_null()),
            "unlisted viewer received the recipient-only value: {:?}",
            seen.system
        );
        assert_eq!(seen.system["open"], 1);
    }
}

#[test]
fn resolve_access_carries_the_resolving_users_id_on_every_branch() {
    let user = test_uuid(0xA1);
    let mut d = doc(PermissionSet::default(), serde_json::json!({}));
    d.permissions.default = crate::data::document::DocRole::Observer;
    assert_eq!(
        resolve_access(user, WorldRole::Player, &d, None).user_id,
        Some(user),
        "main branch"
    );
    // An un-capped GM (no `gm_role`) has no effective role: the short-circuit branch.
    assert!(
        effective_role(user, WorldRole::Gm, &d, None).is_none(),
        "fixture must take the no-role short-circuit"
    );
    assert_eq!(
        resolve_access(user, WorldRole::Gm, &d, None).user_id,
        Some(user),
        "short-circuit branch"
    );
}

#[test]
fn a_gm_is_filtered_like_anyone_when_the_document_holds_a_recipients_tier() {
    let alice = test_uuid(0xA1);
    let gm_id = test_uuid(0x6D);
    let gm = Access {
        all: true,
        see_gm_only: true,
        is_owner: true,
        ..viewer(Some(gm_id))
    };
    let d = doc(
        perms_with(&[
            (
                "/system/secret",
                Visibility::Recipients {
                    recipients: vec![alice],
                },
            ),
            ("/system/gm_note", Visibility::GmOnly),
        ]),
        serde_json::json!({ "secret": "MOCK_SECRET_A", "gm_note": "MOCK_NOTE_A" }),
    );
    let seen = filter_properties(&d, &gm).unwrap();
    assert!(
        seen.system.get("secret").is_none_or(|v| v.is_null()),
        "an unlisted GM received the recipient-only value: {:?}",
        seen.system
    );
    assert_eq!(
        seen.system["gm_note"], "MOCK_NOTE_A",
        "GM-only stays visible"
    );

    let listed = Access {
        user_id: Some(alice),
        ..gm.clone()
    };
    let for_listed_gm = filter_properties(&d, &listed).unwrap();
    assert_eq!(for_listed_gm.system["secret"], "MOCK_SECRET_A");
}

#[test]
fn replay_hides_a_recipients_tier_from_an_unlisted_gm_when_it_was_set_at_commit() {
    assert_replay_hides_from_unlisted_gm(true, false);
}

#[test]
fn replay_hides_a_recipients_tier_from_an_unlisted_gm_when_it_is_set_now() {
    assert_replay_hides_from_unlisted_gm(false, true);
}

/// Replays one `/system/secret` + `/system/open` update to GM recipients.
/// `tier_at_commit` / `tier_now` place a `Recipients([alice])` tier on
/// `/system/secret` in the commit-time snapshot / the current document. An
/// unlisted GM must receive only `/system/open`; the listed GM receives both.
fn assert_replay_hides_from_unlisted_gm(tier_at_commit: bool, tier_now: bool) {
    let world = test_uuid(9);
    let doc_id = test_uuid(2);
    let alice = test_uuid(0xA1);
    let gm_id = test_uuid(0x6D);
    let cmd = Command {
        seq: 1,
        world_id: world,
        author: test_uuid(1),
        ts: 0,
        ops: vec![Operation::Update {
            doc_id,
            changes: ["/system/secret", "/system/open"]
                .into_iter()
                .map(|path| FieldChange {
                    remove: false,
                    path: path.into(),
                    old: serde_json::Value::Null,
                    new: serde_json::json!("v"),
                })
                .collect(),
        }],
    };
    let tiers = vec![(
        "/system/secret".to_string(),
        Visibility::Recipients {
            recipients: vec![alice],
        },
    )];
    let permissions = PermissionSet {
        default: crate::data::document::DocRole::Observer,
        ..Default::default()
    };
    let snapshot = CommandSnapshot {
        per_op: vec![Some(OpSnapshot {
            owner_at_commit: None,
            doc_type: "actor".into(),
            overrides_at_commit: if tier_at_commit {
                tiers.clone()
            } else {
                Vec::new()
            },
            retraction_hidden_at_commit: None,
            created_seq_at_commit: None,
            permissions_at_commit: Some(permissions.clone()),
            permissions_before_commit: None,
            owner_before_commit: None,
        })],
        world_gm_at_commit: HashMap::from([(gm_id, true), (alice, true)]),
    };
    let mut cur = doc(permissions, serde_json::json!({}));
    if tier_now {
        cur.permissions.property_overrides.extend(tiers);
    }
    let current = HashMap::from([(doc_id, current_doc(cur, 0))]);

    let paths_for = |user: uuid::Uuid| -> Vec<String> {
        let ctx = PermissionContext {
            user_id: user,
            world_role: WorldRole::Gm,
        };
        let out = filter_command(
            &cmd,
            &snapshot,
            &ctx,
            &WorldCapDefaults::default(),
            &current,
            |_| None,
        );
        let Operation::Update { changes, .. } = &out.ops[0] else {
            panic!("expected an Update op");
        };
        changes.iter().map(|c| c.path.clone()).collect()
    };
    assert_eq!(
        paths_for(gm_id),
        vec!["/system/open".to_string()],
        "an unlisted GM must not replay the recipient-only change"
    );
    assert_eq!(
        paths_for(alice),
        vec!["/system/secret".to_string(), "/system/open".to_string()],
        "the listed user (here also a GM) replays both"
    );
}
