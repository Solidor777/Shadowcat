use super::*;
use crate::backup::BackupKind;

/// A manifest created at `at` milliseconds.
fn manifest(at: u64) -> BackupManifest {
    BackupManifest {
        shadowcat_version: "0.1.0".into(),
        created_at_unix_ms: at,
        source_db: "shadowcat.db".into(),
        source_assets_dir: "assets".into(),
        asset_file_count: 0,
        db_bytes: 4,
        kind: BackupKind::Manual,
        note: None,
    }
}

/// An in-memory entry for the pure planner.
fn entry(id: &str, at: u64) -> BackupEntry {
    BackupEntry {
        id: id.into(),
        manifest: manifest(at),
        total_bytes: 1,
    }
}

/// Writes a complete backup directory `id` created at `at` under `root`.
fn plant(root: &Path, id: &str, at: u64) {
    let dir = root.join(id);
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    std::fs::write(dir.join("world.db"), b"db").unwrap();
    std::fs::write(dir.join("assets").join("a.bin"), b"abc").unwrap();
    std::fs::write(
        dir.join("manifest.json"),
        serde_json::to_vec(&manifest(at)).unwrap(),
    )
    .unwrap();
}

#[test]
fn list_sorts_newest_first_and_sums_sizes() {
    let tmp = tempfile::tempdir().unwrap();
    plant(tmp.path(), "backup-1", 100);
    plant(tmp.path(), "backup-3", 300);
    plant(tmp.path(), "backup-2", 200);
    let listed = list_backups(tmp.path());
    let ids: Vec<&str> = listed.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, ["backup-3", "backup-2", "backup-1"]);
    let manifest_len = std::fs::metadata(tmp.path().join("backup-1").join("manifest.json"))
        .unwrap()
        .len();
    assert_eq!(listed[0].total_bytes, 2 + 3 + manifest_len);
}

#[test]
fn list_skips_incomplete_and_malformed_children() {
    let tmp = tempfile::tempdir().unwrap();
    plant(tmp.path(), "good", 1);
    // No world.db.
    plant(tmp.path(), "no-db", 2);
    std::fs::rename(
        tmp.path().join("no-db").join("world.db"),
        tmp.path().join("no-db").join("world.db.aside"),
    )
    .unwrap();
    // Malformed manifest.
    plant(tmp.path(), "bad-manifest", 3);
    std::fs::write(tmp.path().join("bad-manifest").join("manifest.json"), b"{").unwrap();
    // A plain file and a directory name outside the id alphabet.
    std::fs::write(tmp.path().join("stray.txt"), b"x").unwrap();
    plant(tmp.path(), "has.dot", 4);
    let ids: Vec<String> = list_backups(tmp.path()).into_iter().map(|e| e.id).collect();
    assert_eq!(ids, ["good"]);
}

#[test]
fn list_of_a_missing_root_is_empty() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(list_backups(&tmp.path().join("absent")).is_empty());
}

#[test]
fn a_manifest_without_kind_or_note_parses_as_manual() {
    let tmp = tempfile::tempdir().unwrap();
    plant(tmp.path(), "old", 5);
    std::fs::write(
        tmp.path().join("old").join("manifest.json"),
        br#"{"shadowcat_version":"0.0.1","created_at_unix_ms":5,"source_db":"d","source_assets_dir":"a","asset_file_count":0,"db_bytes":4}"#,
    )
    .unwrap();
    let listed = list_backups(tmp.path());
    assert_eq!(listed[0].manifest.kind, BackupKind::Manual);
    assert_eq!(listed[0].manifest.note, None);
}

#[test]
fn resolve_accepts_a_direct_child_and_rejects_everything_else() {
    let tmp = tempfile::tempdir().unwrap();
    plant(tmp.path(), "backup-1", 1);
    assert!(resolve_backup(tmp.path(), "backup-1").is_ok());
    let outside = tempfile::tempdir().unwrap();
    for bad in [
        "",
        "..",
        ".",
        "a/b",
        "backup-1/..",
        "..\\x",
        "missing",
        outside.path().to_str().unwrap(),
    ] {
        assert!(
            matches!(
                resolve_backup(tmp.path(), bad),
                Err(BackupError::InvalidBackupDir(..))
            ),
            "{bad:?} must be rejected"
        );
    }
}

#[cfg(unix)]
#[test]
fn resolve_rejects_a_symlink_pointing_outside_the_root() {
    let tmp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), tmp.path().join("link")).unwrap();
    assert!(resolve_backup(tmp.path(), "link").is_err());
}

#[test]
fn plan_prune_keep_last_only() {
    let entries = vec![entry("c", 3), entry("b", 2), entry("a", 1)];
    let policy = BackupRetention {
        keep_last: Some(2),
        keep_days: None,
    };
    assert_eq!(plan_prune(&entries, &policy, 1_000), ["a"]);
}

#[test]
fn plan_prune_keep_days_only() {
    let day = DAY_MS;
    let now = 100 * day;
    let entries = vec![entry("new", now - day), entry("old", now - 5 * day)];
    let policy = BackupRetention {
        keep_last: None,
        keep_days: Some(2),
    };
    assert_eq!(plan_prune(&entries, &policy, now), ["old"]);
}

#[test]
fn plan_prune_with_both_limits_keeps_the_union() {
    let day = DAY_MS;
    let now = 100 * day;
    // `mid` fails the age limit but passes the count limit; `oldest` fails both.
    let entries = vec![
        entry("newest", now - day / 2),
        entry("mid", now - 9 * day),
        entry("oldest", now - 20 * day),
    ];
    let both = BackupRetention {
        keep_last: Some(2),
        keep_days: Some(5),
    };
    assert_eq!(plan_prune(&entries, &both, now), ["oldest"]);
    // The age limit keeps `fresh` even though it is outside the newest 1.
    let fresh = vec![
        entry("newest", now - 1),
        entry("fresh", now - 2),
        entry("stale", now - 8 * day),
    ];
    let policy = BackupRetention {
        keep_last: Some(1),
        keep_days: Some(5),
    };
    assert_eq!(plan_prune(&fresh, &policy, now), ["stale"]);
}

#[test]
fn plan_prune_never_removes_the_newest() {
    let entries = vec![entry("only", 1)];
    let policy = BackupRetention {
        keep_last: Some(0),
        keep_days: Some(0),
    };
    assert!(plan_prune(&entries, &policy, 100 * DAY_MS).is_empty());
    let two = vec![entry("b", 2), entry("a", 1)];
    assert_eq!(plan_prune(&two, &policy, 100 * DAY_MS), ["a"]);
}

#[test]
fn plan_prune_with_no_limits_removes_nothing() {
    let entries = vec![entry("b", 2), entry("a", 1)];
    assert!(plan_prune(&entries, &BackupRetention::default(), u64::MAX).is_empty());
}

#[tokio::test]
async fn prune_moves_the_planned_backups_into_trash() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("backups");
    plant(&root, "backup-1", 100);
    plant(&root, "backup-2", 200);
    plant(&root, "backup-3", 300);
    let trash = Trash::new(
        tmp.path().join("trash"),
        crate::trash::TrashRetention::default(),
    );
    let policy = BackupRetention {
        keep_last: Some(1),
        keep_days: None,
    };
    let mut removed = prune(&root, &policy, 400, &trash).await.unwrap();
    removed.sort();
    assert_eq!(removed, ["backup-1", "backup-2"]);
    let ids: Vec<String> = list_backups(&root).into_iter().map(|e| e.id).collect();
    assert_eq!(ids, ["backup-3"]);
    assert_eq!(trash.list().unwrap().len(), 2);
}
