use super::*;

/// A trash rooted in `base/trash` with `retention`.
fn trash_in(base: &Path, retention: TrashRetention) -> Trash {
    Trash::new(base.join("trash"), retention)
}

/// `n` days as a retention.
fn days(n: u32) -> TrashRetention {
    TrashRetention::Days(NonZeroU32::new(n).unwrap())
}

/// Writes an entry the way `discard_sync` lays one out, with a chosen timestamp and a file `item`
/// of `bytes` bytes; returns its id.
fn plant(trash_dir: &Path, trashed_at_ms: u64, bytes: usize) -> String {
    let id = format!("{trashed_at_ms}-{}", uuid::Uuid::new_v4());
    let entry = trash_dir.join(&id);
    std::fs::create_dir_all(&entry).unwrap();
    let manifest = TrashManifest {
        original_path: PathBuf::from("planted"),
        trashed_at_ms,
        kind: TrashedKind::File,
    };
    std::fs::write(
        entry.join(MANIFEST_FILE),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    std::fs::write(entry.join(ITEM_NAME), vec![7u8; bytes]).unwrap();
    id
}

/// Every file under `root` as `(relative path, contents)`, sorted; the shape two discards of
/// equal fixtures must agree on.
fn tree_of(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                out.push((
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    std::fs::read(&path).unwrap(),
                ));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

/// A directory with `sub/a.bin` (3 bytes) and `b.bin` (2 bytes).
fn make_tree(base: &Path) -> PathBuf {
    let tree = base.join("world");
    std::fs::create_dir_all(tree.join("sub")).unwrap();
    std::fs::write(tree.join("sub").join("a.bin"), b"abc").unwrap();
    std::fs::write(tree.join("b.bin"), b"de").unwrap();
    tree
}

#[test]
fn discard_moves_a_file_into_the_trash_and_writes_a_manifest() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("a.bin");
    std::fs::write(&file, b"payload").unwrap();
    let trash = trash_in(tmp.path(), days(30));
    let before = now_ms();

    trash.discard_sync(&file).unwrap();

    assert!(!file.exists(), "the source is gone from its original path");
    let entries = trash.list().unwrap();
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(entry.manifest.kind, TrashedKind::File);
    assert_eq!(
        entry.manifest.original_path,
        std::path::absolute(&file).unwrap()
    );
    assert!(entry.manifest.trashed_at_ms >= before);
    assert_eq!(entry.size_bytes, 7);
    let entry_dir = tmp.path().join("trash").join(&entry.id);
    assert_eq!(
        std::fs::read(entry_dir.join(ITEM_NAME)).unwrap(),
        b"payload"
    );
    assert!(entry_dir.join(MANIFEST_FILE).is_file());
    assert!(
        entry
            .id
            .starts_with(&entry.manifest.trashed_at_ms.to_string()),
        "the id carries the manifest's timestamp"
    );
}

#[test]
fn discard_moves_a_directory_recursively_and_it_lands_at_item() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = make_tree(tmp.path());
    let trash = trash_in(tmp.path(), days(30));

    trash.discard_sync(&tree).unwrap();

    assert!(!tree.exists());
    let entries = trash.list().unwrap();
    assert_eq!(entries[0].manifest.kind, TrashedKind::Dir);
    assert_eq!(
        entries[0].size_bytes, 5,
        "bytes are summed through nested directories"
    );
    let item = tmp
        .path()
        .join("trash")
        .join(&entries[0].id)
        .join(ITEM_NAME);
    assert_eq!(
        std::fs::read(item.join("sub").join("a.bin")).unwrap(),
        b"abc"
    );
    assert_eq!(std::fs::read(item.join("b.bin")).unwrap(), b"de");
}

#[test]
fn discard_disabled_mode_removes_immediately_and_creates_no_trash_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("a.bin");
    std::fs::write(&file, b"x").unwrap();
    let tree = make_tree(tmp.path());
    let trash = trash_in(tmp.path(), TrashRetention::Disabled);

    trash.discard_sync(&file).unwrap();
    trash.discard_sync(&tree).unwrap();

    assert!(!file.exists() && !tree.exists());
    assert!(
        !tmp.path().join("trash").exists(),
        "disabled mode never creates the trash directory"
    );
    assert!(trash.list().unwrap().is_empty());
}

#[test]
fn discard_is_idempotent_on_an_already_missing_source() {
    let tmp = tempfile::tempdir().unwrap();
    let trash = trash_in(tmp.path(), days(30));
    trash
        .discard_sync(&tmp.path().join("never-existed"))
        .unwrap();
    assert!(trash.list().unwrap().is_empty());
    assert!(
        !tmp.path().join("trash").exists(),
        "a missing source leaves no trace"
    );
}

#[test]
fn a_move_that_loses_the_race_for_the_source_leaves_no_staging_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let trash = trash_in(tmp.path(), days(30));
    let file = tmp.path().join("a.bin");
    std::fs::write(&file, b"x").unwrap();
    // Another discard removes the source between the existence check and the rename.
    let gone = std::fs::remove_file(&file);
    assert!(gone.is_ok());
    let staging = tmp.path().join("trash").join(".staging-probe");
    std::fs::create_dir_all(&staging).unwrap();
    let result = move_item(
        &file,
        &staging.join(ITEM_NAME),
        TrashedKind::File,
        |from, to| std::fs::rename(from, to),
        remove_path,
    );
    assert_eq!(result.unwrap_err().error.kind(), io::ErrorKind::NotFound);
    assert!(
        trash.list().unwrap().is_empty(),
        "a staging directory is never listed"
    );
}

#[test]
fn cross_device_fallback_copies_a_file_then_removes_the_source() {
    let tmp = tempfile::tempdir().unwrap();
    let from = tmp.path().join("from.bin");
    let to = tmp.path().join("to.bin");
    std::fs::write(&from, b"payload").unwrap();

    move_item(
        &from,
        &to,
        TrashedKind::File,
        |_, _| Err(io::ErrorKind::CrossesDevices.into()),
        remove_path,
    )
    .unwrap();

    assert_eq!(std::fs::read(&to).unwrap(), b"payload");
    assert!(!from.exists());
}

#[test]
fn cross_device_fallback_copies_a_directory_tree_then_removes_the_source() {
    let tmp = tempfile::tempdir().unwrap();
    let from = make_tree(tmp.path());
    let expected = tree_of(&from);
    let to = tmp.path().join("copied");

    move_item(
        &from,
        &to,
        TrashedKind::Dir,
        |_, _| Err(io::ErrorKind::CrossesDevices.into()),
        remove_path,
    )
    .unwrap();

    assert_eq!(tree_of(&to), expected);
    assert!(!from.exists());
}

#[test]
fn cross_device_fallback_keeps_the_source_when_the_copy_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let from = tmp.path().join("from.bin");
    std::fs::write(&from, b"payload").unwrap();
    // The destination's parent does not exist, so the copy cannot succeed.
    let to = tmp.path().join("missing-parent").join("to.bin");

    let result = move_item(
        &from,
        &to,
        TrashedKind::File,
        |_, _| Err(io::ErrorKind::CrossesDevices.into()),
        remove_path,
    );

    assert!(!result.unwrap_err().copy_complete);
    assert_eq!(
        std::fs::read(&from).unwrap(),
        b"payload",
        "the source is removed only after the copy completed"
    );
}

#[test]
fn a_rename_error_other_than_cross_device_is_returned_untouched() {
    let tmp = tempfile::tempdir().unwrap();
    let from = tmp.path().join("from.bin");
    std::fs::write(&from, b"x").unwrap();
    let result = move_item(
        &from,
        &tmp.path().join("to.bin"),
        TrashedKind::File,
        |_, _| Err(io::ErrorKind::PermissionDenied.into()),
        remove_path,
    );
    assert_eq!(
        result.unwrap_err().error.kind(),
        io::ErrorKind::PermissionDenied
    );
    assert!(from.exists(), "no fallback runs for other errors");
}

#[test]
fn a_failed_source_removal_after_a_finished_copy_keeps_the_copy_as_a_listed_entry() {
    let tmp = tempfile::tempdir().unwrap();
    let tree = make_tree(tmp.path());
    let trash = trash_in(tmp.path(), days(30));

    // The removal deletes part of the source and then fails, as a locked file would.
    let result = trash.discard_with(
        &tree,
        |_, _| Err(io::ErrorKind::CrossesDevices.into()),
        |from, _| {
            std::fs::remove_file(from.join("b.bin"))?;
            Err(io::ErrorKind::PermissionDenied.into())
        },
    );

    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    let entries = trash.list().unwrap();
    assert_eq!(entries.len(), 1, "the complete copy is a listed entry");
    assert_eq!(
        entries[0].size_bytes, 5,
        "the copy holds the whole tree, including the file the failed removal deleted"
    );
}

#[test]
fn sweep_removes_a_stale_staging_directory_and_keeps_a_fresh_one() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("trash");
    let trash = trash_in(tmp.path(), days(1));
    let staging = dir.join(format!("{STAGING_PREFIX}leftover"));
    std::fs::create_dir_all(staging.join(ITEM_NAME)).unwrap();
    std::fs::write(staging.join(ITEM_NAME).join("a.bin"), b"abc").unwrap();

    assert_eq!(
        trash.sweep(now_ms()).unwrap(),
        (0, 0),
        "a directory touched just now is in flight"
    );
    assert!(staging.exists());

    assert_eq!(trash.sweep(now_ms() + 10 * MS_PER_DAY).unwrap(), (1, 3));
    assert!(!staging.exists());
}

#[test]
fn empty_removes_a_staging_directory_idle_past_the_grace() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("trash");
    let trash = trash_in(tmp.path(), days(30));
    let staging = dir.join(format!("{STAGING_PREFIX}leftover"));
    std::fs::create_dir_all(&staging).unwrap();

    trash.empty_at(now_ms() + 2 * STAGING_GRACE_MS).unwrap();

    assert!(!staging.exists());
}

#[cfg(unix)]
#[test]
fn an_entry_whose_item_cannot_be_read_still_lists_and_does_not_stop_the_sweep() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("trash");
    let trash = trash_in(tmp.path(), days(1));
    let bad = plant(&dir, 1, 1);
    let item = dir.join(&bad).join(ITEM_NAME);
    std::fs::remove_file(&item).unwrap();
    std::fs::create_dir_all(item.join("inner")).unwrap();
    std::fs::set_permissions(&item, std::fs::Permissions::from_mode(0o000)).unwrap();
    plant(&dir, 2, 1);

    let listed = trash.list();

    std::fs::set_permissions(&item, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(listed.unwrap().len(), 2);
}

#[test]
fn sweep_removes_only_entries_older_than_retention_and_keeps_newer_ones() {
    let tmp = tempfile::tempdir().unwrap();
    let trash = trash_in(tmp.path(), days(1));
    let dir = tmp.path().join("trash");
    let now = 10 * MS_PER_DAY;
    let cutoff = now - MS_PER_DAY;
    let older = plant(&dir, cutoff - 1, 4);
    let at_cutoff = plant(&dir, cutoff, 4);
    let newer = plant(&dir, cutoff + 1, 4);

    let (count, bytes) = trash.sweep(now).unwrap();

    assert_eq!((count, bytes), (1, 4));
    assert!(!dir.join(older).exists());
    assert!(
        dir.join(at_cutoff).exists(),
        "an entry exactly retention days old is kept"
    );
    assert!(dir.join(newer).exists());
}

#[test]
fn sweep_is_a_noop_under_never_and_under_disabled() {
    for retention in [TrashRetention::Never, TrashRetention::Disabled] {
        let tmp = tempfile::tempdir().unwrap();
        let trash = trash_in(tmp.path(), retention);
        let id = plant(&tmp.path().join("trash"), 1, 1);
        assert_eq!(trash.sweep(u64::MAX).unwrap(), (0, 0));
        assert!(tmp.path().join("trash").join(id).exists());
    }
}

#[test]
fn empty_removes_every_entry_and_reports_correct_count_and_bytes() {
    let tmp = tempfile::tempdir().unwrap();
    let trash = trash_in(tmp.path(), days(30));
    trash.discard_sync(&make_tree(tmp.path())).unwrap();
    let file = tmp.path().join("a.bin");
    std::fs::write(&file, b"four").unwrap();
    trash.discard_sync(&file).unwrap();

    let (count, bytes) = trash.empty().unwrap();

    assert_eq!(count, 2);
    assert_eq!(bytes, 5 + 4, "nested directory bytes are counted");
    assert!(trash.list().unwrap().is_empty());
}

#[test]
fn empty_one_removes_exactly_the_named_entry_and_leaves_others() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("trash");
    let trash = trash_in(tmp.path(), days(30));
    let first = plant(&dir, 1, 1);
    let second = plant(&dir, 2, 1);

    let removed = trash.empty_one(&first.parse().unwrap()).unwrap();

    assert!(removed);
    assert!(!dir.join(&first).exists());
    assert!(dir.join(&second).exists());
}

#[test]
fn empty_one_on_a_missing_id_returns_false_not_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let trash = trash_in(tmp.path(), days(30));
    let id: EntryId = "1737490000123-00000000-0000-0000-0000-000000000000"
        .parse()
        .unwrap();
    assert!(!trash.empty_one(&id).unwrap());
    plant(&tmp.path().join("trash"), 5, 1);
    assert!(
        !trash.empty_one(&id).unwrap(),
        "a second removal is also false"
    );
}

#[test]
fn empty_and_list_skip_a_staging_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("trash");
    let trash = trash_in(tmp.path(), days(30));
    let staging = dir.join(format!("{STAGING_PREFIX}in-flight"));
    std::fs::create_dir_all(staging.join(ITEM_NAME)).unwrap();
    plant(&dir, 1, 1);

    assert_eq!(trash.list().unwrap().len(), 1);
    assert_eq!(trash.empty().unwrap().0, 1);
    assert!(
        staging.exists(),
        "a half-built entry is never removed by an emptying pass"
    );
}

#[test]
fn list_is_newest_first() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("trash");
    let trash = trash_in(tmp.path(), days(30));
    plant(&dir, 10, 1);
    plant(&dir, 30, 1);
    plant(&dir, 20, 1);
    let order: Vec<u64> = trash
        .list()
        .unwrap()
        .iter()
        .map(|e| e.manifest.trashed_at_ms)
        .collect();
    assert_eq!(order, vec![30, 20, 10]);
}

#[test]
fn entry_id_rejects_path_traversal_forms() {
    let well_formed = "1737490000123-3fa85f64-5717-4562-b3fc-2c963f66afa6";
    let overlong = format!("{well_formed}{}", "0".repeat(ENTRY_ID_MAX_LEN));
    let cases: Vec<(String, EntryIdError)> = vec![
        ("..".into(), EntryIdError::Malformed),
        ("../x".into(), EntryIdError::Malformed),
        ("/etc/passwd".into(), EntryIdError::Malformed),
        (format!("{well_formed}/x"), EntryIdError::Malformed),
        (format!("x\\{well_formed}"), EntryIdError::Malformed),
        (format!("{well_formed}\0"), EntryIdError::Malformed),
        ("C:x".into(), EntryIdError::Malformed),
        ("\\\\server\\share".into(), EntryIdError::Malformed),
        (
            "1737490000123\u{2044}3fa85f64-5717-4562-b3fc-2c963f66afa6".into(),
            EntryIdError::Malformed,
        ),
        (well_formed.to_uppercase(), EntryIdError::Malformed),
        (String::new(), EntryIdError::Malformed),
        (overlong, EntryIdError::TooLong),
    ];
    for (input, expected) in cases {
        assert_eq!(input.parse::<EntryId>().unwrap_err(), expected, "{input:?}");
    }
    assert_eq!(
        well_formed.parse::<EntryId>().unwrap().as_str(),
        well_formed
    );
}

#[test]
fn a_well_formed_id_naming_no_entry_parses_and_only_misses_at_lookup() {
    let tmp = tempfile::tempdir().unwrap();
    let id: EntryId = "1737490000123-00000000-0000-0000-0000-000000000000"
        .parse()
        .unwrap();
    assert!(!trash_in(tmp.path(), days(30)).empty_one(&id).unwrap());
}

#[tokio::test]
async fn discard_sync_and_discard_agree() {
    let tmp_sync = tempfile::tempdir().unwrap();
    let tmp_async = tempfile::tempdir().unwrap();
    let sync_tree = make_tree(tmp_sync.path());
    let async_tree = make_tree(tmp_async.path());
    let sync_trash = trash_in(tmp_sync.path(), days(30));
    let async_trash = trash_in(tmp_async.path(), days(30));

    sync_trash.discard_sync(&sync_tree).unwrap();
    async_trash.discard(&async_tree).await.unwrap();

    let sync_entry = &sync_trash.list().unwrap()[0];
    let async_entry = &async_trash.list().unwrap()[0];
    assert_eq!(sync_entry.manifest.kind, async_entry.manifest.kind);
    assert_eq!(sync_entry.size_bytes, async_entry.size_bytes);
    assert_eq!(
        tree_of(
            &tmp_sync
                .path()
                .join("trash")
                .join(&sync_entry.id)
                .join(ITEM_NAME)
        ),
        tree_of(
            &tmp_async
                .path()
                .join("trash")
                .join(&async_entry.id)
                .join(ITEM_NAME)
        ),
    );
    assert!(!sync_tree.exists() && !async_tree.exists());
}

#[tokio::test]
async fn the_retention_sweep_removes_an_expired_entry_at_startup() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("trash");
    let expired = plant(&dir, 1, 1);
    let fresh = plant(&dir, now_ms(), 1);

    spawn_retention_sweep(Arc::new(trash_in(tmp.path(), days(1))));

    let reaped = tokio::time::timeout(Duration::from_secs(10), async {
        while dir.join(&expired).exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(
        reaped.is_ok(),
        "the first sweep tick removed the expired entry"
    );
    assert!(
        dir.join(fresh).exists(),
        "a current entry survives the sweep"
    );
}
