//! Key-file protection: the failure paths and the "never touch a directory we did not create"
//! rule.

use super::*;

#[test]
fn a_protection_failure_aborts_the_write_and_leaves_nothing_behind() {
    let base = tempfile::tempdir().unwrap();
    let key = base.path().join("tls").join("key.pem");
    let err = write_private_with(&key, b"SECRET", |_| {
        Err(std::io::Error::other("acl refused"))
    })
    .unwrap_err();
    assert!(err.to_string().contains("acl refused"));
    assert!(
        !key.exists(),
        "no key file may exist after a refused protection"
    );
    let mut tmp = key.file_name().unwrap().to_os_string();
    tmp.push(".tmp");
    let tmp = key.with_file_name(tmp);
    assert!(!tmp.exists(), "the temporary file is cleaned up");
}

#[test]
fn no_secret_byte_is_written_before_protection_succeeds() {
    let base = tempfile::tempdir().unwrap();
    let key = base.path().join("key.pem");
    write_private_with(&key, b"SECRET", |tmp| {
        assert_eq!(
            std::fs::metadata(tmp).unwrap().len(),
            0,
            "protection runs while the file is still empty"
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(std::fs::read(&key).unwrap(), b"SECRET");
}

#[test]
fn a_failed_rename_cleans_up_the_temporary_file() {
    let base = tempfile::tempdir().unwrap();
    // The destination is a non-empty directory, so the final rename cannot succeed.
    let key = base.path().join("key.pem");
    std::fs::create_dir_all(key.join("inner")).unwrap();
    std::fs::write(key.join("inner").join("f"), b"x").unwrap();
    assert!(write_private(&key, b"SECRET").is_err());
    assert!(!base.path().join("key.pem.tmp").exists());
}

#[cfg(unix)]
#[test]
fn an_existing_directory_keeps_its_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let base = tempfile::tempdir().unwrap();
    let dir = base.path().join("operator-chosen");
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    write_private(&dir.join("key.pem"), b"SECRET").unwrap();
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        mode(&dir),
        0o755,
        "an operator-chosen directory is left alone"
    );
    assert_eq!(
        mode(&dir.join("key.pem")),
        0o600,
        "the key itself is protected"
    );
}

#[cfg(unix)]
#[test]
fn only_missing_directories_are_restricted() {
    use std::os::unix::fs::PermissionsExt;
    let base = tempfile::tempdir().unwrap();
    let existing = base.path().join("existing");
    std::fs::create_dir(&existing).unwrap();
    std::fs::set_permissions(&existing, std::fs::Permissions::from_mode(0o755)).unwrap();
    let leaf = existing.join("a").join("tls");
    ensure_private_dir(&leaf).unwrap();
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&existing), 0o755);
    assert_eq!(mode(&existing.join("a")), 0o700);
    assert_eq!(mode(&leaf), 0o700);
}

#[cfg(windows)]
#[test]
fn an_existing_directory_keeps_its_acl_and_a_created_one_is_protected() {
    let base = tempfile::tempdir().unwrap();
    let existing = base.path().join("operator-chosen");
    std::fs::create_dir(&existing).unwrap();
    let before = dacl_sddl(&existing).unwrap();
    write_private(&existing.join("key.pem"), b"SECRET").unwrap();
    assert_eq!(
        dacl_sddl(&existing).unwrap(),
        before,
        "an operator-chosen directory's ACL is left alone"
    );
    let created = base.path().join("made-here");
    ensure_private_dir(&created).unwrap();
    assert!(dacl_sddl(&created).unwrap().starts_with("D:P"));
}
