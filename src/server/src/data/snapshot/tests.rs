use super::*;
use crate::data::command::Operation;

fn sample_command() -> Command {
    Command {
        seq: 7,
        world_id: Uuid::from_u128(1),
        author: Uuid::from_u128(2),
        ts: 100,
        ops: vec![Operation::Update {
            doc_id: Uuid::from_u128(3),
            changes: vec![],
        }],
    }
}

#[test]
fn stored_command_round_trips_through_json() {
    let stored = StoredCommand {
        command: sample_command(),
        snapshot: CommandSnapshot {
            per_op: vec![Some(OpSnapshot {
                owner_at_commit: Some(Uuid::from_u128(4)),
                doc_type: "actor".into(),
                overrides_at_commit: vec![("/system/secret".into(), Visibility::GmOnly)],
                retraction_hidden_at_commit: None,
                created_seq_at_commit: Some(5),
                permissions_at_commit: Some(PermissionSet::default()),
                permissions_before_commit: Some(PermissionSet::default()),
                owner_before_commit: Some(Uuid::from_u128(6)),
            })],
            world_gm_at_commit: HashMap::from([(Uuid::from_u128(4), true)]),
        },
    };
    let s = serde_json::to_string(&stored).unwrap();
    let back = StoredCommand::from_stored_json(&s).unwrap();
    assert_eq!(stored, back);
}

#[test]
fn from_stored_json_falls_back_for_a_legacy_bare_command_row() {
    let cmd = sample_command();
    let raw = serde_json::to_string(&cmd).unwrap();
    let stored = StoredCommand::from_stored_json(&raw).unwrap();
    assert_eq!(stored.command, cmd);
    assert_eq!(stored.snapshot.per_op, vec![None]);
    assert!(stored.snapshot.world_gm_at_commit.is_empty());
}

#[test]
fn from_stored_json_rejects_genuinely_malformed_json() {
    assert!(StoredCommand::from_stored_json("{not json").is_err());
}
