//! Recoverable server-side deletion. Production code removes a file or directory through
//! [`Trash::discard`] / [`Trash::discard_sync`] and never through a raw `remove_file` /
//! `remove_dir_all`: the item moves into a trash directory beside the data, and a periodic sweep
//! ([`spawn_retention_sweep`]) or an admin action ([`Trash::empty`], [`Trash::empty_one`]) is the
//! only thing that removes it for good. [`TrashRetention::Disabled`] bypasses the trash and removes
//! the item at once.
//!
//! Directory layout, one directory per discarded item:
//!
//! ```text
//! <trash_dir>/<trashed_at_ms>-<uuid>/manifest.json   original path, time, kind
//! <trash_dir>/<trashed_at_ms>-<uuid>/item            the discarded file or directory
//! ```
//!
//! An entry is built under a `.staging-<uuid>` name nothing else reads and renamed into its final
//! name as the last step, so [`Trash::list`], [`Trash::sweep`] and [`Trash::empty`] never see a
//! half-built entry. Recovery is manual: move `item` back to the manifest's `original_path`.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::io;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::config::Config;

/// Milliseconds in one day; the unit `TrashRetention::Days` counts in.
const MS_PER_DAY: u64 = 86_400_000;

/// How often [`spawn_retention_sweep`] runs a pass.
const SWEEP_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// File name of an entry's manifest inside its entry directory.
const MANIFEST_FILE: &str = "manifest.json";

/// Name of the discarded item inside its entry directory.
const ITEM_NAME: &str = "item";

/// Prefix of the directory an entry is built in before it is renamed to its final name.
/// [`EntryId`] never matches it, so listings skip it.
const STAGING_PREFIX: &str = ".staging-";

/// The longest string [`EntryId`] parses; a cheap bound checked before the regex runs.
const ENTRY_ID_MAX_LEN: usize = 64;

/// Exactly the shape [`Trash::discard_sync`] names an entry: a 1 to 20 digit millisecond timestamp,
/// a hyphen, and a lowercase hyphenated UUID. One positive allowlist, so `..`, separators of
/// either kind, a NUL, a drive prefix and look-alike Unicode separators are all rejected by the
/// same match.
static ENTRY_ID_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"^[0-9]{1,20}-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")
        .expect("ENTRY_ID_RE is a valid constant pattern")
});

/// How long a discarded item survives in the trash before the retention sweep removes it.
///
/// Serialized as a bare integer (days), `"never"` or `"disabled"`; deserialized from those, and
/// from a numeric string (environment variables arrive as strings).
///
/// # Examples
///
/// ```
/// use shadowcat::trash::TrashRetention;
///
/// assert_eq!("never".parse::<TrashRetention>().unwrap(), TrashRetention::Never);
/// assert!("0".parse::<TrashRetention>().is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrashRetention {
    /// Sweep entries older than this many days.
    Days(NonZeroU32),
    /// Never sweep; entries accumulate until an admin empties the trash.
    Never,
    /// [`Trash::discard`] bypasses the trash: a plain, unrecoverable remove, and no trash
    /// directory is created.
    Disabled,
}

impl TrashRetention {
    /// The retention a configuration that does not name one gets.
    pub const DEFAULT_DAYS: u32 = 30;
}

impl Default for TrashRetention {
    fn default() -> Self {
        Self::Days(
            NonZeroU32::new(Self::DEFAULT_DAYS).expect("DEFAULT_DAYS is a positive constant"),
        )
    }
}

/// A value that is not one of the three accepted retention spellings.
///
/// # Examples
///
/// ```
/// use shadowcat::trash::TrashRetention;
///
/// let err = "sometimes".parse::<TrashRetention>().unwrap_err();
/// assert!(err.to_string().contains("\"never\""));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TrashRetentionError {
    /// Zero or a negative day count; immediate deletion is spelled `"disabled"`.
    #[error("trash_retention {0} is not a positive number of days; write \"disabled\" to delete immediately")]
    NotPositive(i64),
    /// A day count larger than a `u32` holds.
    #[error("trash_retention {0} days is too large")]
    TooLarge(i64),
    /// Anything else.
    #[error("trash_retention {0:?} must be a positive number of days, \"never\" or \"disabled\"")]
    Unrecognized(String),
}

impl TrashRetention {
    /// The retention for a signed day count; shared by the integer and string parse paths.
    fn from_days(days: i64) -> Result<Self, TrashRetentionError> {
        if days <= 0 {
            return Err(TrashRetentionError::NotPositive(days));
        }
        let days = u32::try_from(days).map_err(|_| TrashRetentionError::TooLarge(days))?;
        NonZeroU32::new(days)
            .map(Self::Days)
            .ok_or(TrashRetentionError::NotPositive(0))
    }
}

impl FromStr for TrashRetention {
    type Err = TrashRetentionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "never" => Ok(Self::Never),
            "disabled" => Ok(Self::Disabled),
            other => match other.parse::<i64>() {
                Ok(days) => Self::from_days(days),
                Err(_) => Err(TrashRetentionError::Unrecognized(other.to_string())),
            },
        }
    }
}

impl Serialize for TrashRetention {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Days(days) => serializer.serialize_u32(days.get()),
            Self::Never => serializer.serialize_str("never"),
            Self::Disabled => serializer.serialize_str("disabled"),
        }
    }
}

/// Reads a retention from a TOML integer, an environment-variable string, or the two keywords.
struct RetentionVisitor;

impl Visitor<'_> for RetentionVisitor {
    type Value = TrashRetention;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a positive number of days, \"never\" or \"disabled\"")
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
        let days =
            i64::try_from(v).map_err(|_| E::custom(TrashRetentionError::TooLarge(i64::MAX)))?;
        TrashRetention::from_days(days).map_err(E::custom)
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
        TrashRetention::from_days(v).map_err(E::custom)
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
        v.parse().map_err(E::custom)
    }
}

impl<'de> Deserialize<'de> for TrashRetention {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(RetentionVisitor)
    }
}

/// Whether a discarded item was a file or a directory.
///
/// # Examples
///
/// ```
/// use shadowcat::trash::TrashedKind;
///
/// assert_eq!(serde_json::to_string(&TrashedKind::Dir).unwrap(), "\"dir\"");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrashedKind {
    /// A regular file.
    File,
    /// A directory tree.
    Dir,
}

/// One entry's on-disk record, `manifest.json` beside the discarded `item`. Never sent to a
/// client as is; the admin listing reshapes it.
///
/// # Examples
///
/// ```
/// use shadowcat::trash::{TrashManifest, TrashedKind};
///
/// let m = TrashManifest {
///     original_path: std::path::PathBuf::from("data").join("assets").join("a.png"),
///     trashed_at_ms: 1_700_000_000_000,
///     kind: TrashedKind::File,
/// };
/// let back: TrashManifest = serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
/// assert_eq!(back, m);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrashManifest {
    /// Absolute path the item had when it was discarded; where a manual restore puts it back.
    pub original_path: PathBuf,
    /// Unix milliseconds at which the discard ran.
    pub trashed_at_ms: u64,
    /// Shape of `item`.
    pub kind: TrashedKind,
}

/// One entry as [`Trash::list`] reports it.
///
/// # Examples
///
/// ```
/// let dir = tempfile::tempdir().unwrap();
/// let trash = shadowcat::trash::Trash::new(
///     dir.path().join("trash"),
///     shadowcat::trash::TrashRetention::default(),
/// );
/// assert!(trash.list().unwrap().is_empty());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashEntry {
    /// The entry directory's name, `<trashed_at_ms>-<uuid>`: the stable id [`Trash::empty_one`]
    /// takes, parsed back through [`EntryId`] at any boundary that receives one.
    pub id: String,
    /// The entry's manifest.
    pub manifest: TrashManifest,
    /// Bytes under `item`: the file length, or the sum over a directory tree.
    pub size_bytes: u64,
}

/// A trash entry id that has been proven to name only a direct child of the trash directory.
///
/// Grammar, as one regex: `^[0-9]{1,20}-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$`,
/// and at most 64 bytes. A real `now_millis` value is 13 digits until the year 2286, so the
/// 20-digit bound never rejects a genuine id. The match is a positive allowlist: no path
/// separator, `..`, NUL, drive prefix or look-alike separator can satisfy it, so a value that
/// parses cannot escape the trash directory when joined onto it.
///
/// # Examples
///
/// ```
/// use shadowcat::trash::EntryId;
///
/// let id: EntryId = "1737490000123-3fa85f64-5717-4562-b3fc-2c963f66afa6".parse().unwrap();
/// assert_eq!(id.as_str(), "1737490000123-3fa85f64-5717-4562-b3fc-2c963f66afa6");
/// assert!("../x".parse::<EntryId>().is_err());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryId(String);

/// Why a string is not an [`EntryId`].
///
/// # Examples
///
/// ```
/// use shadowcat::trash::{EntryId, EntryIdError};
///
/// assert_eq!("x".parse::<EntryId>().unwrap_err(), EntryIdError::Malformed);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EntryIdError {
    /// Longer than 64 bytes; rejected before any pattern matching.
    #[error("trash entry id is too long")]
    TooLong,
    /// Not of the `<timestamp>-<uuid>` shape.
    #[error("trash entry id is malformed")]
    Malformed,
}

impl EntryId {
    /// The validated id text.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::trash::EntryId;
    ///
    /// let id: EntryId = "1-00000000-0000-0000-0000-000000000000".parse().unwrap();
    /// assert_eq!(id.as_str(), "1-00000000-0000-0000-0000-000000000000");
    /// ```
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for EntryId {
    type Err = EntryIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() > ENTRY_ID_MAX_LEN {
            return Err(EntryIdError::TooLong);
        }
        if !ENTRY_ID_RE.is_match(s) {
            return Err(EntryIdError::Malformed);
        }
        Ok(Self(s.to_string()))
    }
}

/// The one place production code removes files and directories. Owned by `AppState` behind an
/// `Arc`, and built directly by the one-shot backup and restore modes, which have no `AppState`.
///
/// # Examples
///
/// ```
/// use shadowcat::trash::{Trash, TrashRetention};
///
/// let dir = tempfile::tempdir().unwrap();
/// let file = dir.path().join("scratch.bin");
/// std::fs::write(&file, b"x").unwrap();
/// let trash = Trash::new(dir.path().join("trash"), TrashRetention::default());
/// trash.discard_sync(&file).unwrap();
/// assert!(!file.exists());
/// assert_eq!(trash.list().unwrap().len(), 1);
/// ```
#[derive(Debug, Clone)]
pub struct Trash {
    /// Directory holding the entries; created on the first discard.
    dir: PathBuf,
    /// Governs whether a discard trashes and whether a sweep removes.
    retention: TrashRetention,
}

/// Unix milliseconds now, from the crate's one time source for persisted records.
fn now_ms() -> u64 {
    u64::try_from(crate::auth::setup::now_millis()).unwrap_or(0)
}

/// Removes `path` for good; a path that is already gone is not an error. The only permanent
/// removal in this module, reached by the `Disabled` mode, the cleanup of a copied source, and the
/// operations that empty the trash.
fn remove_path(path: &Path, kind: TrashedKind) -> io::Result<()> {
    let result = match kind {
        TrashedKind::Dir => std::fs::remove_dir_all(path),
        TrashedKind::File => std::fs::remove_file(path),
    };
    match result {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// Copies the directory tree `from` into `to`. Symlinks are not followed or copied, so a tree
/// that holds one loses the link (never its target) when the copy-then-remove fallback runs.
fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let target = to.join(entry.file_name());
        if file_type.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if file_type.is_file() {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Moves `from` to `to` with `rename`; when `rename` reports the two paths are on different
/// devices, copies `from` to `to` and only then removes `from`. The source is removed last so a
/// failed or interrupted copy leaves it where it was. `rename` is a parameter so a test can
/// force the cross-device branch without two filesystems.
fn move_item(
    from: &Path,
    to: &Path,
    kind: TrashedKind,
    rename: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> io::Result<()> {
    match rename(from, to) {
        Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
            match kind {
                TrashedKind::Dir => copy_tree(from, to)?,
                TrashedKind::File => {
                    std::fs::copy(from, to)?;
                }
            }
            remove_path(from, kind)
        }
        other => other,
    }
}

/// Bytes under `path`: the length of a file, the sum over a directory tree, 0 when absent.
/// Symlinks count as their own length and are not followed.
fn size_of(path: &Path) -> io::Result<u64> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    if !meta.is_dir() {
        return Ok(meta.len());
    }
    let mut total = 0u64;
    for entry in std::fs::read_dir(path)? {
        total = total.saturating_add(size_of(&entry?.path())?);
    }
    Ok(total)
}

impl Trash {
    /// A trash rooted at `dir` with the given `retention`.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::trash::{Trash, TrashRetention};
    ///
    /// let trash = Trash::new(std::path::PathBuf::from("trash"), TrashRetention::Never);
    /// assert!(trash.list().unwrap().is_empty());
    /// ```
    pub fn new(dir: PathBuf, retention: TrashRetention) -> Self {
        Self { dir, retention }
    }

    /// The trash the configuration describes: [`Config::trash_path`] and
    /// [`Config::trash_retention`]. The one constructor from a `Config`, so `main`, `AppState`
    /// and the one-shot backup and restore modes agree.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::config::Config;
    /// use shadowcat::trash::Trash;
    ///
    /// let trash = Trash::from_config(&Config::default());
    /// assert!(trash.list().unwrap().is_empty());
    /// ```
    pub fn from_config(config: &Config) -> Self {
        Self::new(config.trash_path(), config.trash_retention)
    }

    /// Moves `path` (file or directory) into the trash, or removes it at once under
    /// [`TrashRetention::Disabled`]. A path that no longer exists is `Ok`. Runs
    /// [`Trash::discard_sync`] on the blocking pool; there is no second implementation.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::trash::{Trash, TrashRetention};
    ///
    /// # #[tokio::main] async fn main() {
    /// let dir = tempfile::tempdir().unwrap();
    /// let file = dir.path().join("gone.bin");
    /// std::fs::write(&file, b"x").unwrap();
    /// let trash = Trash::new(dir.path().join("trash"), TrashRetention::default());
    /// trash.discard(&file).await.unwrap();
    /// assert!(!file.exists());
    /// # }
    /// ```
    pub async fn discard(&self, path: &Path) -> io::Result<()> {
        let this = self.clone();
        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || this.discard_sync(&path))
            .await
            .map_err(io::Error::other)?
    }

    /// [`Trash::discard`] for a caller already on the blocking pool, or in a `Drop` that must
    /// finish before it returns.
    ///
    /// Ordering keeps a failure from losing the item: the entry is built under a staging name,
    /// the manifest is written, then the item is moved in (renamed, or copied then the source
    /// removed on a cross-device rename), and only then is the staging directory renamed to the
    /// entry id. A failure before the last rename removes the staging directory and returns the
    /// error with the source untouched. A failure of the last rename keeps the staging directory,
    /// because it holds the only copy.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::trash::{Trash, TrashRetention};
    ///
    /// let dir = tempfile::tempdir().unwrap();
    /// let tree = dir.path().join("world");
    /// std::fs::create_dir_all(tree.join("sub")).unwrap();
    /// std::fs::write(tree.join("sub").join("a.bin"), b"abc").unwrap();
    /// let trash = Trash::new(dir.path().join("trash"), TrashRetention::default());
    /// trash.discard_sync(&tree).unwrap();
    /// assert!(!tree.exists());
    /// assert_eq!(trash.list().unwrap()[0].size_bytes, 3);
    /// ```
    pub fn discard_sync(&self, path: &Path) -> io::Result<()> {
        let kind = match std::fs::symlink_metadata(path) {
            Ok(m) if m.is_dir() => TrashedKind::Dir,
            Ok(_) => TrashedKind::File,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };
        if self.retention == TrashRetention::Disabled {
            return remove_path(path, kind);
        }
        std::fs::create_dir_all(&self.dir)?;
        let trashed_at_ms = now_ms();
        let staging = self
            .dir
            .join(format!("{STAGING_PREFIX}{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&staging)?;
        let staged = (|| -> io::Result<bool> {
            let manifest = TrashManifest {
                original_path: std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()),
                trashed_at_ms,
                kind,
            };
            let bytes = serde_json::to_vec_pretty(&manifest).map_err(io::Error::other)?;
            std::fs::write(staging.join(MANIFEST_FILE), bytes)?;
            match move_item(path, &staging.join(ITEM_NAME), kind, |from, to| {
                std::fs::rename(from, to)
            }) {
                Ok(()) => Ok(true),
                // Another discard of the same path moved it first: nothing left to trash.
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
                Err(e) => Err(e),
            }
        })();
        match staged {
            Ok(true) => {
                let id = format!("{trashed_at_ms}-{}", uuid::Uuid::new_v4());
                std::fs::rename(&staging, self.dir.join(id))
            }
            Ok(false) => remove_path(&staging, TrashedKind::Dir),
            Err(e) => {
                let _ = remove_path(&staging, TrashedKind::Dir);
                Err(e)
            }
        }
    }

    /// Every entry in the trash, newest first. Empty when the trash holds nothing or, under
    /// [`TrashRetention::Disabled`], does not exist. A directory whose name is not an entry id
    /// (a staging directory) and an entry whose manifest cannot be read are skipped.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::trash::{Trash, TrashRetention};
    ///
    /// let dir = tempfile::tempdir().unwrap();
    /// let trash = Trash::new(dir.path().join("trash"), TrashRetention::default());
    /// assert!(trash.list().unwrap().is_empty());
    /// ```
    pub fn list(&self) -> io::Result<Vec<TrashEntry>> {
        let read = match std::fs::read_dir(&self.dir) {
            Ok(r) => r,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let mut entries = Vec::new();
        for dirent in read {
            let dirent = dirent?;
            let Some(id) = dirent
                .file_name()
                .to_str()
                .filter(|name| name.parse::<EntryId>().is_ok())
                .map(str::to_string)
            else {
                continue;
            };
            let entry_dir = dirent.path();
            let manifest = std::fs::read(entry_dir.join(MANIFEST_FILE)).and_then(|b| {
                serde_json::from_slice::<TrashManifest>(&b).map_err(io::Error::other)
            });
            let manifest = match manifest {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(%id, error = %e, "trash entry has no readable manifest");
                    continue;
                }
            };
            let size_bytes = size_of(&entry_dir.join(ITEM_NAME))?;
            entries.push(TrashEntry {
                id,
                manifest,
                size_bytes,
            });
        }
        entries.sort_by(|a, b| {
            b.manifest
                .trashed_at_ms
                .cmp(&a.manifest.trashed_at_ms)
                .then_with(|| b.id.cmp(&a.id))
        });
        Ok(entries)
    }

    /// Removes the listed entries for good and returns `(count, bytes)` removed.
    fn remove_entries(&self, entries: &[TrashEntry]) -> io::Result<(u64, u64)> {
        let (mut count, mut bytes) = (0u64, 0u64);
        for entry in entries {
            remove_path(&self.dir.join(&entry.id), TrashedKind::Dir)?;
            count += 1;
            bytes = bytes.saturating_add(entry.size_bytes);
        }
        Ok((count, bytes))
    }

    /// Permanently removes every entry; returns `(count, bytes)` removed.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::trash::{Trash, TrashRetention};
    ///
    /// let dir = tempfile::tempdir().unwrap();
    /// let file = dir.path().join("a.bin");
    /// std::fs::write(&file, b"four").unwrap();
    /// let trash = Trash::new(dir.path().join("trash"), TrashRetention::default());
    /// trash.discard_sync(&file).unwrap();
    /// assert_eq!(trash.empty().unwrap(), (1, 4));
    /// ```
    pub fn empty(&self) -> io::Result<(u64, u64)> {
        self.remove_entries(&self.list()?)
    }

    /// Permanently removes the entry `id`. `Ok(false)` when no such entry exists (a concurrent
    /// removal got there first).
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::trash::{EntryId, Trash, TrashRetention};
    ///
    /// let dir = tempfile::tempdir().unwrap();
    /// let trash = Trash::new(dir.path().join("trash"), TrashRetention::default());
    /// let id: EntryId = "1-00000000-0000-0000-0000-000000000000".parse().unwrap();
    /// assert!(!trash.empty_one(&id).unwrap());
    /// ```
    pub fn empty_one(&self, id: &EntryId) -> io::Result<bool> {
        let path = self.dir.join(id.as_str());
        match std::fs::symlink_metadata(&path) {
            Ok(_) => {
                remove_path(&path, TrashedKind::Dir)?;
                Ok(true)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// One retention pass: removes every entry trashed more than the retention's day count before
    /// `now_ms` and returns `(count, bytes)` removed. A no-op under `Never` and `Disabled`.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::trash::{Trash, TrashRetention};
    ///
    /// let dir = tempfile::tempdir().unwrap();
    /// let trash = Trash::new(dir.path().join("trash"), TrashRetention::Never);
    /// assert_eq!(trash.sweep(u64::MAX).unwrap(), (0, 0));
    /// ```
    pub fn sweep(&self, now_ms: u64) -> io::Result<(u64, u64)> {
        let TrashRetention::Days(days) = self.retention else {
            return Ok((0, 0));
        };
        let cutoff = now_ms.saturating_sub(u64::from(days.get()).saturating_mul(MS_PER_DAY));
        let expired: Vec<TrashEntry> = self
            .list()?
            .into_iter()
            .filter(|e| e.manifest.trashed_at_ms < cutoff)
            .collect();
        self.remove_entries(&expired)
    }
}

/// Starts the hourly retention sweep: the first pass runs at once, so a restart does not wait an
/// hour; a failed pass logs at `warn` and the loop keeps ticking. Starts nothing under
/// [`TrashRetention::Disabled`], which has no directory to sweep. Never called from the one-shot
/// backup and restore modes.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
/// use shadowcat::trash::{spawn_retention_sweep, Trash, TrashRetention};
///
/// # #[tokio::main] async fn main() {
/// let dir = tempfile::tempdir().unwrap();
/// let trash = Arc::new(Trash::new(dir.path().join("trash"), TrashRetention::Never));
/// spawn_retention_sweep(trash);
/// # }
/// ```
pub fn spawn_retention_sweep(trash: Arc<Trash>) {
    if trash.retention == TrashRetention::Disabled {
        return;
    }
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(SWEEP_INTERVAL);
        loop {
            tick.tick().await;
            let pass = Arc::clone(&trash);
            match tokio::task::spawn_blocking(move || pass.sweep(now_ms())).await {
                Ok(Ok((count, bytes))) if count > 0 => {
                    tracing::info!(
                        count,
                        bytes,
                        "trash retention sweep removed expired entries"
                    );
                }
                Ok(Ok(_)) => {}
                Ok(Err(e)) => tracing::warn!(error = %e, "trash retention sweep failed"),
                Err(e) => tracing::warn!(error = %e, "trash retention sweep task failed"),
            }
        }
    });
}

#[cfg(test)]
mod tests;
