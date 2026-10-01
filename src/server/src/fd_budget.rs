//! The process's open-descriptor budget: raising the soft `RLIMIT_NOFILE` at startup and fitting
//! the connection caps inside what remains.
//!
//! WHY: every admitted connection holds one descriptor, and both connection caps
//! ([`Limits::global_cap`] before hand-off, [`ConnLimits::max_live`] after it) are chosen
//! bounds, not measured capacities. A soft limit below their sum (the common Unix default is 1024)
//! lets a flood exhaust descriptors, after which `accept` fails and the server cannot open its
//! database, assets or outbound fetches either. The budget makes the caps structurally unable to
//! promise more descriptors than the process may hold.
//!
//! INVARIANTS:
//! 1. Both caps derive from ONE budget, the effective soft limit read back with `getrlimit` AFTER
//!    the raise attempt minus [`FD_RESERVE`], never from the value the raise asked for: a failed
//!    or partial raise must constrain the caps the same way a limit that was never raisable does.
//! 2. A descriptor is held by a connection either before hand-off or after it, never both, but a
//!    connection is admitted to the next stage before the previous one releases, so the caps are
//!    fitted to their SUM.
//! 3. The platform ceiling is queried at runtime (macOS `kern.maxfilesperproc`), never a
//!    remembered literal, because `setrlimit` on macOS rejects a soft limit above it even when the
//!    hard limit is unlimited.
//! 4. A platform without `RLIMIT_NOFILE` reports no limit and the caps are not clamped; no ceiling
//!    is claimed for it.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use crate::http::conn_serve::ConnLimits;
use crate::tls::listener::Limits;

/// Descriptors held back from the connection caps for everything that is not a client
/// connection: the database and its write-ahead log, asset and backup files, the log file, the
/// listeners, outbound fetches (link previews, module downloads) and the runtime's own handles.
/// Chosen, not measured: generous against a table's handful of concurrent file operations, and
/// small against any raised limit.
pub const FD_RESERVE: u64 = 512;

/// Floor of [`Limits::global_cap`] after clamping. A server that admits nothing is not serving;
/// when the descriptor limit is at or below [`FD_RESERVE`] the floors win over the budget and the
/// startup warning says so.
pub const MIN_PRE_HANDOFF_CAP: usize = 16;

/// Floor of [`ConnLimits::max_live`] after clamping (see [`MIN_PRE_HANDOFF_CAP`]); larger than
/// the pre-hand-off floor because one player holds several live connections.
pub const MIN_LIVE_CAP: usize = 32;

/// What the soft `RLIMIT_NOFILE` is after startup's raise attempt.
///
/// # Examples
///
/// ```
/// let limit = shadowcat::fd_budget::raise_nofile_limit();
/// // `None` is "no limit to budget against" (unbounded, or a platform without the limit).
/// if let Some(soft) = limit.soft {
///     assert!(soft > 0);
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NofileLimit {
    /// The effective soft limit read back after the raise attempt; `None` when it is unbounded
    /// or when the platform has no such limit.
    pub soft: Option<u64>,
    /// The soft limit before the raise attempt, when the platform reports one.
    pub before: Option<u64>,
    /// Why the limit could not be raised or read, when that happened.
    pub problem: Option<String>,
}

/// What a clamp changed, for the one startup warning.
///
/// # Examples
///
/// ```
/// use shadowcat::fd_budget::fit_to_descriptor_budget;
/// use shadowcat::http::conn_serve::ConnLimits;
/// use shadowcat::tls::listener::Limits;
///
/// let (_, _, clamp) = fit_to_descriptor_budget(Some(1024), Limits::default(), ConnLimits::default());
/// let clamp = clamp.expect("1024 descriptors cannot back the default caps");
/// assert_eq!(clamp.soft, 1024);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clamp {
    /// The soft limit the caps were fitted to.
    pub soft: u64,
    /// `soft - FD_RESERVE`, saturating at zero.
    pub budget: u64,
    /// [`Limits::global_cap`] before the clamp.
    pub pre_handoff_before: usize,
    /// [`Limits::global_cap`] after the clamp.
    pub pre_handoff_after: usize,
    /// [`ConnLimits::max_live`] before the clamp.
    pub live_before: usize,
    /// [`ConnLimits::max_live`] after the clamp.
    pub live_after: usize,
    /// Whether the floors exceed the budget, so descriptors can still run out.
    pub floors_exceed_budget: bool,
}

/// Fits both connection caps inside `soft - FD_RESERVE` descriptors, keeping their ratio, and
/// returns the limits to use plus what changed (`None` when nothing had to). `soft` is the
/// effective soft limit, `None` for no limit. Per-peer caps never exceed their global cap.
///
/// Algorithm: proportional scaling of the two global caps to their shared budget, floored at
/// [`MIN_PRE_HANDOFF_CAP`] and [`MIN_LIVE_CAP`]. Proportional, not first-come, so neither stage
/// can starve the other: a flood of half-open handshakes cannot consume the descriptors the
/// live tables need, and the reverse.
///
/// # Examples
///
/// ```
/// use shadowcat::fd_budget::{fit_to_descriptor_budget, FD_RESERVE};
/// use shadowcat::http::conn_serve::ConnLimits;
/// use shadowcat::tls::listener::Limits;
///
/// let (pre, live, clamp) =
///     fit_to_descriptor_budget(Some(1024), Limits::default(), ConnLimits::default());
/// assert!(clamp.is_some());
/// assert!(pre.global_cap as u64 + live.max_live as u64 <= 1024 - FD_RESERVE);
///
/// let (pre, live, clamp) = fit_to_descriptor_budget(None, Limits::default(), ConnLimits::default());
/// assert!(clamp.is_none());
/// assert_eq!((pre.global_cap, live.max_live), (1024, 2048));
/// ```
pub fn fit_to_descriptor_budget(
    soft: Option<u64>,
    pre: Limits,
    live: ConnLimits,
) -> (Limits, ConnLimits, Option<Clamp>) {
    let Some(soft) = soft else {
        return (pre, live, None);
    };
    let budget = soft.saturating_sub(FD_RESERVE);
    let wanted = pre.global_cap as u128 + live.max_live as u128;
    if u128::from(budget) >= wanted {
        return (pre, live, None);
    }
    let scale = |cap: usize, floor: usize| -> usize {
        let scaled = u128::from(budget) * cap as u128 / wanted.max(1);
        (scaled as usize).max(floor).min(cap)
    };
    let mut fitted_pre = pre;
    let mut fitted_live = live;
    fitted_pre.global_cap = scale(pre.global_cap, MIN_PRE_HANDOFF_CAP);
    fitted_live.max_live = scale(live.max_live, MIN_LIVE_CAP);
    fitted_pre.per_ip_cap = pre
        .per_ip_cap
        .min(u32::try_from(fitted_pre.global_cap).unwrap_or(u32::MAX));
    fitted_live.max_live_per_peer = live
        .max_live_per_peer
        .min(u32::try_from(fitted_live.max_live).unwrap_or(u32::MAX));
    let clamp = Clamp {
        soft,
        budget,
        pre_handoff_before: pre.global_cap,
        pre_handoff_after: fitted_pre.global_cap,
        live_before: live.max_live,
        live_after: fitted_live.max_live,
        floors_exceed_budget: (fitted_pre.global_cap + fitted_live.max_live) as u128
            > u128::from(budget),
    };
    (fitted_pre, fitted_live, Some(clamp))
}

/// Raises the soft `RLIMIT_NOFILE` to the hard limit (bounded by the platform ceiling) and returns
/// the soft limit read back afterwards. A refused raise is logged at `warn` and the current
/// limit is used.
///
/// # Examples
///
/// ```
/// let limit = shadowcat::fd_budget::raise_nofile_limit();
/// assert!(limit.soft.map_or(true, |soft| soft > 0));
/// ```
#[cfg(unix)]
pub fn raise_nofile_limit() -> NofileLimit {
    let Some((before_soft, hard)) = read_nofile() else {
        let problem = format!(
            "getrlimit(RLIMIT_NOFILE) failed: {}",
            std::io::Error::last_os_error()
        );
        tracing::warn!("{problem}; the connection caps are not fitted to a descriptor limit");
        return NofileLimit {
            soft: None,
            before: None,
            problem: Some(problem),
        };
    };
    let target = raise_target(before_soft, hard, platform_ceiling());
    let mut problem = None;
    if target > before_soft {
        if let Err(e) = write_nofile_soft(target, hard) {
            let message = format!(
                "raising the open-file limit from {} to {target} failed: {e}",
                describe(before_soft)
            );
            tracing::warn!("{message}; continuing with the current limit");
            problem = Some(message);
        }
    }
    let after = read_nofile().map(|(soft, _)| soft);
    if problem.is_none() && target > before_soft {
        tracing::info!(
            from = %describe(before_soft),
            to = %after.map_or_else(|| "unknown".to_string(), describe),
            "raised the open-file limit"
        );
    }
    NofileLimit {
        soft: after.and_then(bounded),
        before: bounded(before_soft),
        problem,
    }
}

/// On a platform without `RLIMIT_NOFILE` there is no limit to raise and none to budget against:
/// the caps are not clamped, and no descriptor ceiling is claimed for the platform.
///
/// # Examples
///
/// ```
/// let limit = shadowcat::fd_budget::raise_nofile_limit();
/// assert!(limit.soft.map_or(true, |soft| soft > 0));
/// ```
#[cfg(not(unix))]
pub fn raise_nofile_limit() -> NofileLimit {
    NofileLimit {
        soft: None,
        before: None,
        problem: None,
    }
}

/// The soft limit a raise should ask for: the hard limit, bounded by the platform `ceiling`, and
/// never below the current soft limit.
///
/// # Examples
///
/// ```
/// use shadowcat::fd_budget::raise_target;
///
/// assert_eq!(raise_target(1024, 65_536, None), 65_536);
/// assert_eq!(raise_target(1024, u64::MAX, Some(10_240)), 10_240);
/// assert_eq!(raise_target(20_000, 65_536, Some(10_240)), 20_000);
/// ```
pub fn raise_target(soft: u64, hard: u64, ceiling: Option<u64>) -> u64 {
    let wanted = ceiling.map_or(hard, |c| hard.min(c));
    wanted.max(soft)
}

/// Widens a platform `rlim_t` (32 or 64 bits depending on the target) to `u64` without a cast
/// that is redundant on the targets where it is already `u64`.
#[cfg(unix)]
fn widen<T: Into<u64>>(value: T) -> u64 {
    value.into()
}

/// The soft limit as a budget input: `None` for an unbounded limit.
#[cfg(unix)]
fn bounded(limit: u64) -> Option<u64> {
    (limit != widen(libc::RLIM_INFINITY)).then_some(limit)
}

/// A limit for a log line.
#[cfg(unix)]
fn describe(limit: u64) -> String {
    match bounded(limit) {
        Some(n) => n.to_string(),
        None => "unlimited".to_string(),
    }
}

/// Reads `(soft, hard)` of `RLIMIT_NOFILE`.
#[cfg(unix)]
fn read_nofile() -> Option<(u64, u64)> {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `getrlimit(2)` (POSIX.1-2008) writes one `rlimit` through the pointer; `limit` is a
    // live, exclusively borrowed, correctly typed local.
    let status = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) };
    (status == 0).then_some((widen(limit.rlim_cur), widen(limit.rlim_max)))
}

/// Sets the soft `RLIMIT_NOFILE` to `soft`, leaving the hard limit at `hard`.
#[cfg(unix)]
fn write_nofile_soft(soft: u64, hard: u64) -> std::io::Result<()> {
    let limit = libc::rlimit {
        rlim_cur: soft as libc::rlim_t,
        rlim_max: hard as libc::rlim_t,
    };
    // SAFETY: `setrlimit(2)` (POSIX.1-2008) reads one `rlimit` through the pointer; `limit` is a
    // live, correctly typed local.
    let status = unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) };
    if status == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// The highest soft limit `setrlimit` accepts on this platform, when it is lower than the hard
/// limit can say. macOS caps it at `kern.maxfilesperproc` regardless of an unlimited hard limit
/// (`setrlimit(2)` on Darwin, EINVAL); Linux accepts any value up to the hard limit.
///
/// # Examples
///
/// ```
/// // `Some` only where the platform has such a ceiling; it is queried, never assumed.
/// let ceiling = shadowcat::fd_budget::platform_ceiling();
/// if let Some(c) = ceiling {
///     assert!(c > 0);
/// }
/// ```
#[cfg(target_os = "macos")]
pub fn platform_ceiling() -> Option<u64> {
    let mut value: libc::c_int = 0;
    let mut len = std::mem::size_of::<libc::c_int>();
    // SAFETY: `sysctlbyname(3)` with a NUL-terminated name writes at most `len` bytes into
    // `value`, which is a live `c_int`, and updates `len`; no new value is passed.
    let status = unsafe {
        libc::sysctlbyname(
            c"kern.maxfilesperproc".as_ptr(),
            (&mut value as *mut libc::c_int).cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    (status == 0 && value > 0).then_some(value as u64)
}

/// The highest soft limit `setrlimit` accepts on this platform; Linux and the other Unixes
/// accept any value up to the hard limit, and a platform without the limit has no ceiling to
/// claim.
///
/// # Examples
///
/// ```
/// // `Some` only where the platform has such a ceiling; it is queried, never assumed.
/// let ceiling = shadowcat::fd_budget::platform_ceiling();
/// if let Some(c) = ceiling {
///     assert!(c > 0);
/// }
/// ```
#[cfg(not(target_os = "macos"))]
pub fn platform_ceiling() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests;
