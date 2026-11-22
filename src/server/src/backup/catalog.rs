//! This module lists, resolves and prunes the backups under one backups root.
//!
//! Every function takes a root directory and nothing else from the server, so the admin routes,
//! the pre-restore step and a later scheduler share one implementation. An id reaches the
//! filesystem only through [`resolve_backup`]; a deletion goes through [`Trash`] and never
//! removes a directory outright.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{BackupError, BackupManifest};
use crate::config::Config;
use crate::trash::Trash;

/// Milliseconds in one day, the unit [`BackupRetention::keep_days`] converts through.
const DAY_MS: u64 = 86_400_000;

/// One backup directory as the admin list shows it.
///
/// # Examples
///
/// ```
/// use shadowcat::backup::catalog::BackupEntry;
/// use shadowcat::backup::{BackupKind, BackupManifest};
///
/// let entry = BackupEntry {
///     id: "backup-1".into(),
///     manifest: BackupManifest {
///         shadowcat_version: "0.1.0".into(),
///         created_at_unix_ms: 1,
///         source_db: "shadowcat.db".into(),
///         source_assets_dir: "assets".into(),
///         asset_file_count: 0,
///         db_bytes: 10,
///         kind: BackupKind::Manual,
///         note: None,
///     },
///     total_bytes: 10,
/// };
/// assert_eq!(serde_json::to_value(&entry).unwrap()["id"], "backup-1");
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct BackupEntry {
    /// The directory name under the backups root; the only handle a route accepts.
    pub id: String,
    /// The parsed `manifest.json`.
    pub manifest: BackupManifest,
    /// Sum of every file's size under the backup directory (database plus assets).
    pub total_bytes: u64,
}

/// How many backups to keep. Both limits `None` keeps every backup.
///
/// # Examples
///
/// ```
/// use shadowcat::backup::catalog::BackupRetention;
/// use shadowcat::config::Config;
///
/// let cfg = Config { backup_keep_last: Some(3), ..Config::default() };
/// let policy = BackupRetention::from_config(&cfg);
/// assert_eq!(policy.keep_last, Some(3));
/// assert_eq!(policy.keep_days, None);
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct BackupRetention {
    /// Keep the newest this-many backups.
    pub keep_last: Option<u32>,
    /// Keep backups younger than this many days.
    pub keep_days: Option<u32>,
}

impl BackupRetention {
    /// The policy `config`'s `backup_keep_last` and `backup_keep_days` describe.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::backup::catalog::BackupRetention;
    /// use shadowcat::config::Config;
    ///
    /// assert_eq!(BackupRetention::from_config(&Config::default()), BackupRetention::default());
    /// ```
    pub fn from_config(config: &Config) -> Self {
        Self {
            keep_last: config.backup_keep_last,
            keep_days: config.backup_keep_days,
        }
    }

    /// True when neither limit is set, so pruning can never remove anything.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::backup::catalog::BackupRetention;
    ///
    /// assert!(BackupRetention::default().is_unlimited());
    /// assert!(!BackupRetention { keep_last: Some(1), keep_days: None }.is_unlimited());
    /// ```
    pub fn is_unlimited(&self) -> bool {
        self.keep_last.is_none() && self.keep_days.is_none()
    }
}

/// The file name of the staged-restore marker, written beside the database file.
pub const RESTORE_PENDING_FILE: &str = "restore-pending.json";

/// The request the admin restore route leaves for the next server start: which backup to
/// restore, and when it was asked for.
///
/// # Examples
///
/// ```
/// use shadowcat::backup::catalog::RestorePending;
///
/// let marker = RestorePending { id: "backup-1".into(), requested_at_ms: 5 };
/// let json = serde_json::to_string(&marker).unwrap();
/// assert_eq!(serde_json::from_str::<RestorePending>(&json).unwrap(), marker);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestorePending {
    /// The backup directory name under the backups root.
    pub id: String,
    /// When the restore was requested, Unix epoch milliseconds.
    pub requested_at_ms: u64,
}

/// The staged-restore marker's path: [`RESTORE_PENDING_FILE`] in the database file's directory.
/// The marker is read only from the server's own data directory, never from a request.
///
/// # Examples
///
/// ```
/// use shadowcat::backup::catalog::restore_pending_path;
///
/// let path = restore_pending_path(std::path::Path::new("data").join("shadowcat.db").as_path());
/// assert_eq!(path, std::path::Path::new("data").join("restore-pending.json"));
/// ```
pub fn restore_pending_path(db_path: &Path) -> PathBuf {
    db_path.with_file_name(RESTORE_PENDING_FILE)
}

/// True when `id` is a non-empty run of ASCII letters, digits, `_` and `-`: a single path
/// component with no separator, dot or drive prefix.
///
/// # Examples
///
/// ```text
/// id_is_well_formed("backup-17")? == true
/// id_is_well_formed("../x")? == false
/// ```
fn id_is_well_formed(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Sum of the sizes of every regular file under `dir`, skipping symlinks.
///
/// # Examples
///
/// ```text
/// dir_bytes(&backup_dir)? // -> db bytes + asset bytes + manifest bytes
/// ```
fn dir_bytes(dir: &Path) -> std::io::Result<u64> {
    let mut total = 0u64;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            total = total.saturating_add(dir_bytes(&entry.path())?);
        } else if file_type.is_file() {
            total = total.saturating_add(entry.metadata()?.len());
        }
    }
    Ok(total)
}

/// Reads one child of the backups root as an entry, or `None` when it is not a valid backup
/// (not a directory, no `world.db`, missing or malformed manifest, unreadable).
///
/// # Examples
///
/// ```text
/// read_entry(&root.join("backup-17")) // -> Some(entry) for a complete backup
/// ```
fn read_entry(path: &Path) -> Option<BackupEntry> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.is_dir() {
        return None;
    }
    let id = path.file_name()?.to_str()?.to_string();
    if !id_is_well_formed(&id) {
        return None;
    }
    std::fs::metadata(path.join("world.db")).ok()?;
    let bytes = std::fs::read(path.join("manifest.json")).ok()?;
    let manifest: BackupManifest = serde_json::from_slice(&bytes).ok()?;
    let total_bytes = dir_bytes(path).ok()?;
    Some(BackupEntry {
        id,
        manifest,
        total_bytes,
    })
}

/// Every valid backup directly under `root`, newest first (ties broken by id, descending). A
/// missing root lists nothing. A child that is not a complete backup is skipped, not an error.
///
/// # Examples
///
/// ```
/// use shadowcat::backup::catalog::list_backups;
///
/// let dir = tempfile::tempdir().unwrap();
/// assert!(list_backups(dir.path()).is_empty());
/// ```
pub fn list_backups(root: &Path) -> Vec<BackupEntry> {
    let Ok(children) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut entries: Vec<BackupEntry> = children
        .filter_map(|c| c.ok())
        .filter_map(|c| read_entry(&c.path()))
        .collect();
    entries.sort_by(|a, b| {
        b.manifest
            .created_at_unix_ms
            .cmp(&a.manifest.created_at_unix_ms)
            .then_with(|| b.id.cmp(&a.id))
    });
    entries
}

/// The directory of backup `id` under `root`. The only way a route turns an id into a path: the
/// id must be a single `[A-Za-z0-9_-]+` component naming a real (non-symlink) directory whose
/// canonical parent is the canonical `root`; anything else is
/// [`BackupError::InvalidBackupDir`].
///
/// # Examples
///
/// ```
/// use shadowcat::backup::catalog::resolve_backup;
///
/// let dir = tempfile::tempdir().unwrap();
/// std::fs::create_dir(dir.path().join("backup-1")).unwrap();
/// assert!(resolve_backup(dir.path(), "backup-1").is_ok());
/// assert!(resolve_backup(dir.path(), "..").is_err());
/// ```
pub fn resolve_backup(root: &Path, id: &str) -> Result<PathBuf, BackupError> {
    let invalid = |why: &str| BackupError::InvalidBackupDir(id.to_string(), why.to_string());
    if !id_is_well_formed(id) {
        return Err(invalid("malformed backup id"));
    }
    let candidate = root.join(id);
    let meta = std::fs::symlink_metadata(&candidate).map_err(|_| invalid("no such backup"))?;
    if !meta.is_dir() {
        return Err(invalid("not a backup directory"));
    }
    let canonical = candidate
        .canonicalize()
        .map_err(|_| invalid("no such backup"))?;
    let canonical_root = root
        .canonicalize()
        .map_err(|_| invalid("backups root is unreadable"))?;
    if canonical.parent() != Some(canonical_root.as_path()) {
        return Err(invalid("backup is outside the backups root"));
    }
    Ok(canonical)
}

/// The ids `policy` removes from `entries` at time `now_ms`. Pure: the newest entry is never
/// planned. With both limits set an entry is kept when either limit keeps it (the union);
/// with one set only that limit applies; with neither set nothing is planned.
///
/// # Examples
///
/// ```
/// use shadowcat::backup::catalog::{plan_prune, BackupEntry, BackupRetention};
/// use shadowcat::backup::{BackupKind, BackupManifest};
///
/// let entry = |id: &str, at: u64| BackupEntry {
///     id: id.into(),
///     manifest: BackupManifest {
///         shadowcat_version: "0.1.0".into(),
///         created_at_unix_ms: at,
///         source_db: "shadowcat.db".into(),
///         source_assets_dir: "assets".into(),
///         asset_file_count: 0,
///         db_bytes: 1,
///         kind: BackupKind::Manual,
///         note: None,
///     },
///     total_bytes: 1,
/// };
/// let entries = vec![entry("b", 2), entry("a", 1)];
/// let policy = BackupRetention { keep_last: Some(1), keep_days: None };
/// assert_eq!(plan_prune(&entries, &policy, 10), vec!["a".to_string()]);
/// ```
pub fn plan_prune(entries: &[BackupEntry], policy: &BackupRetention, now_ms: u64) -> Vec<String> {
    if policy.is_unlimited() {
        return Vec::new();
    }
    let mut ordered: Vec<&BackupEntry> = entries.iter().collect();
    ordered.sort_by(|a, b| {
        b.manifest
            .created_at_unix_ms
            .cmp(&a.manifest.created_at_unix_ms)
            .then_with(|| b.id.cmp(&a.id))
    });
    let cutoff = policy
        .keep_days
        .map(|d| now_ms.saturating_sub(u64::from(d).saturating_mul(DAY_MS)));
    ordered
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(index, entry)| {
            let kept_by_count = policy.keep_last.is_some_and(|n| *index < n as usize);
            let kept_by_age = cutoff.is_some_and(|c| entry.manifest.created_at_unix_ms >= c);
            !(kept_by_count || kept_by_age)
        })
        .map(|(_, entry)| entry.id.clone())
        .collect()
}

/// Moves every backup [`plan_prune`] selects into `trash` and returns the removed ids. Never
/// removes a directory outright.
///
/// # Errors
///
/// The first failed discard; backups already moved stay moved.
///
/// # Examples
///
/// ```
/// # #[tokio::main] async fn main() {
/// use shadowcat::backup::catalog::{prune, BackupRetention};
/// use shadowcat::trash::{Trash, TrashRetention};
///
/// let dir = tempfile::tempdir().unwrap();
/// let trash = Trash::new(dir.path().join("trash"), TrashRetention::default());
/// let removed = prune(dir.path(), &BackupRetention::default(), 0, &trash).await.unwrap();
/// assert!(removed.is_empty());
/// # }
/// ```
pub async fn prune(
    root: &Path,
    policy: &BackupRetention,
    now_ms: u64,
    trash: &Trash,
) -> Result<Vec<String>, BackupError> {
    let listing_root = root.to_path_buf();
    let entries = tokio::task::spawn_blocking(move || list_backups(&listing_root))
        .await
        .map_err(std::io::Error::other)?;
    let planned = plan_prune(&entries, policy, now_ms);
    let mut removed = Vec::with_capacity(planned.len());
    for id in planned {
        let dir = resolve_backup(root, &id)?;
        trash.discard(&dir).await?;
        removed.push(id);
    }
    Ok(removed)
}

#[cfg(test)]
mod tests;
