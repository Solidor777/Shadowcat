//! Owner-only private key files: mode 0600 on Unix, a protected owner-only DACL on Windows, in a
//! directory restricted the same way WHEN this module created it.
//!
//! INVARIANTS:
//! 1. A key file is never written unprotected: [`write_private`] restricts the still-empty
//!    temporary file BEFORE any secret byte is written and returns the error when the OS refuses,
//!    so the caller degrades to an in-memory certificate (`selfsigned::load_or_generate`'s
//!    `persist_error`) instead of persisting a key under an inherited ACL.
//! 2. Directories the operator already had are never re-permissioned: rewriting the ACL or mode of
//!    an operator-chosen `tls_dir` (or one of its ancestors) could lock out backup agents or the
//!    operator's own account. Only directories this module creates are restricted; a failure to
//!    restrict one of those is logged and non-fatal because the key file's own protection is what
//!    guards the secret.
//! 3. A failed write leaves no `<key>.tmp` behind.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::io::Write;
use std::path::Path;

/// Creates `dir` (and any missing parents) and restricts every directory it CREATED to the
/// current user. Directories that already existed are left exactly as they are.
///
/// # Examples
///
/// ```
/// let base = tempfile::tempdir().unwrap();
/// let dir = base.path().join("tls");
/// shadowcat::tls::keyfile::ensure_private_dir(&dir).unwrap();
/// assert!(dir.is_dir());
/// ```
pub fn ensure_private_dir(dir: &Path) -> std::io::Result<()> {
    let mut created = Vec::new();
    let mut cursor = Some(dir);
    while let Some(d) = cursor.filter(|d| !d.as_os_str().is_empty() && !d.exists()) {
        created.push(d);
        cursor = d.parent();
    }
    std::fs::create_dir_all(dir)?;
    for d in created.into_iter().rev() {
        protect_dir(d);
    }
    Ok(())
}

/// Writes `contents` to `path` readable only by the current user: the bytes go to a sibling
/// temporary file that is protected BEFORE any secret byte is written, then renamed over `path`
/// so a reader never observes a partial key. Fails (writing nothing secret, leaving no temporary
/// file) when the file cannot be protected.
///
/// # Examples
///
/// ```
/// let base = tempfile::tempdir().unwrap();
/// let path = base.path().join("tls").join("key.pem");
/// shadowcat::tls::keyfile::write_private(&path, b"secret").unwrap();
/// assert_eq!(std::fs::read(&path).unwrap(), b"secret");
/// ```
pub fn write_private(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    write_private_with(path, contents, protect_file)
}

/// [`write_private`] with the protection step injected, so a refusing OS can be simulated.
fn write_private_with(
    path: &Path,
    contents: &[u8],
    protect: impl FnOnce(&Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        ensure_private_dir(parent)?;
    }
    let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(".tmp");
    let tmp = path.with_file_name(tmp_name);
    let result = (|| {
        let mut file = open_private(&tmp)?;
        protect(&tmp)?;
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        // Best effort: the temporary file may hold secret bytes.
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Opens `path` for writing, truncating, with owner-only mode on Unix.
#[cfg(unix)]
fn open_private(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
}

/// Opens `path` for writing, truncating; the DACL is applied by `protect_file` right after.
#[cfg(not(unix))]
fn open_private(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
}

/// Restricts `dir` to the current user, logging (never failing) when the OS refuses.
#[cfg(unix)]
fn protect_dir(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Err(e) = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)) {
        tracing::warn!(path = %dir.display(), error = %e, "could not restrict TLS directory permissions");
    }
}

/// Restricts `file` to the current user.
#[cfg(unix)]
fn protect_file(file: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600))
}

/// Restricts `dir` to the current user through a protected DACL whose ACE is inherited by
/// children.
#[cfg(windows)]
fn protect_dir(dir: &Path) {
    if let Err(e) = win::protect(dir, true) {
        tracing::warn!(path = %dir.display(), error = %e, "could not restrict TLS directory ACL");
    }
}

/// Restricts `file` to the current user through a protected owner-only DACL.
#[cfg(windows)]
fn protect_file(file: &Path) -> std::io::Result<()> {
    win::protect(file, false)
}

/// The SDDL text of `path`'s DACL (Windows only), for verifying protection.
///
/// # Examples
///
/// ```
/// # #[cfg(windows)] {
/// let dir = tempfile::tempdir().unwrap();
/// let p = dir.path().join("key.pem");
/// shadowcat::tls::keyfile::write_private(&p, b"x").unwrap();
/// let sddl = shadowcat::tls::keyfile::dacl_sddl(&p).unwrap();
/// assert!(sddl.starts_with("D:P"));
/// # }
/// ```
#[cfg(windows)]
pub fn dacl_sddl(path: &Path) -> std::io::Result<String> {
    win::dacl_sddl(path)
}

#[cfg(windows)]
mod win {
    //! Windows ACL plumbing over the `windows` crate.

    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::{CloseHandle, LocalFree, ERROR_SUCCESS, HANDLE, HLOCAL};
    use windows::Win32::Security::Authorization::{
        ConvertSecurityDescriptorToStringSecurityDescriptorW, ConvertSidToStringSidW,
        ConvertStringSecurityDescriptorToSecurityDescriptorW, GetNamedSecurityInfoW,
        SetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT,
    };
    use windows::Win32::Security::{
        GetSecurityDescriptorDacl, GetTokenInformation, TokenUser, ACL, DACL_SECURITY_INFORMATION,
        PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, TOKEN_QUERY, TOKEN_USER,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    /// NUL-terminated UTF-16 form of `s`.
    fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
        s.encode_wide().chain(std::iter::once(0)).collect()
    }

    /// Converts a crate error to `io::Error`.
    fn io(e: windows::core::Error) -> std::io::Error {
        std::io::Error::other(e.to_string())
    }

    /// The current user's SID in string form (`S-1-5-21-...`).
    fn current_user_sid() -> std::io::Result<String> {
        // SAFETY: plain Win32 calls; every out-pointer refers to a live local, the token handle
        // is closed on every path, and the SID string buffer is freed with LocalFree.
        unsafe {
            let mut token = HANDLE::default();
            OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).map_err(io)?;
            let mut needed = 0u32;
            let _ = GetTokenInformation(token, TokenUser, None, 0, &mut needed);
            let mut buf = vec![0u8; needed as usize];
            let got = GetTokenInformation(
                token,
                TokenUser,
                Some(buf.as_mut_ptr().cast()),
                needed,
                &mut needed,
            );
            let _ = CloseHandle(token);
            got.map_err(io)?;
            // `buf` is a byte vector with no alignment guarantee for `TOKEN_USER`, so the header is
            // copied out with an unaligned read; the SID it points at lives inside `buf`.
            let user = std::ptr::read_unaligned(buf.as_ptr().cast::<TOKEN_USER>());
            let mut sid_str = PWSTR::null();
            ConvertSidToStringSidW(user.User.Sid, &mut sid_str).map_err(io)?;
            let text = sid_str.to_string().map_err(std::io::Error::other);
            let _ = LocalFree(HLOCAL(sid_str.0.cast()));
            text
        }
    }

    /// Replaces `path`'s DACL with one ACE granting the current user full control and marks it
    /// protected (no inheritance from the parent). `container` makes the ACE inheritable by
    /// children of a directory.
    pub(super) fn protect(path: &Path, container: bool) -> std::io::Result<()> {
        let sid = current_user_sid()?;
        let flags = if container { "OICI" } else { "" };
        let sddl = format!("D:P(A;{flags};FA;;;{sid})");
        let sddl_w = wide(std::ffi::OsStr::new(&sddl));
        let path_w = wide(path.as_os_str());
        // SAFETY: `sddl_w`/`path_w` are NUL-terminated and outlive the calls; the descriptor
        // allocated by the conversion is freed with LocalFree; the DACL pointer is only used
        // while that descriptor is alive.
        unsafe {
            let mut sd = PSECURITY_DESCRIPTOR::default();
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sddl_w.as_ptr()),
                SDDL_REVISION_1,
                &mut sd,
                None,
            )
            .map_err(io)?;
            let mut present = false.into();
            let mut defaulted = false.into();
            let mut dacl: *mut ACL = std::ptr::null_mut();
            let got = GetSecurityDescriptorDacl(sd, &mut present, &mut dacl, &mut defaulted);
            let result = match got {
                Ok(()) => SetNamedSecurityInfoW(
                    PCWSTR(path_w.as_ptr()),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                    PSID::default(),
                    PSID::default(),
                    Some(dacl),
                    None,
                ),
                Err(e) => {
                    let _ = LocalFree(HLOCAL(sd.0));
                    return Err(io(e));
                }
            };
            let _ = LocalFree(HLOCAL(sd.0));
            if result != ERROR_SUCCESS {
                return Err(std::io::Error::from_raw_os_error(result.0 as i32));
            }
        }
        Ok(())
    }

    /// SDDL of `path`'s DACL.
    pub(super) fn dacl_sddl(path: &Path) -> std::io::Result<String> {
        let path_w = wide(path.as_os_str());
        // SAFETY: out-pointers refer to live locals; both allocated buffers are freed with
        // LocalFree after use.
        unsafe {
            let mut sd = PSECURITY_DESCRIPTOR::default();
            let rc = GetNamedSecurityInfoW(
                PCWSTR(path_w.as_ptr()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                None,
                None,
                &mut sd,
            );
            if rc != ERROR_SUCCESS {
                return Err(std::io::Error::from_raw_os_error(rc.0 as i32));
            }
            let mut out = PWSTR::null();
            let conv = ConvertSecurityDescriptorToStringSecurityDescriptorW(
                sd,
                SDDL_REVISION_1,
                DACL_SECURITY_INFORMATION,
                &mut out,
                None,
            );
            let text = match conv {
                Ok(()) => out.to_string().map_err(std::io::Error::other),
                Err(e) => Err(io(e)),
            };
            if !out.is_null() {
                let _ = LocalFree(HLOCAL(out.0.cast()));
            }
            let _ = LocalFree(HLOCAL(sd.0));
            text
        }
    }
}

#[cfg(test)]
mod tests;
