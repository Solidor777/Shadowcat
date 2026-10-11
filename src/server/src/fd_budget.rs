//! Open-descriptor budget: raising `RLIMIT_NOFILE` and fitting the connection caps to it.
//!
//! The process's open-descriptor budget: raising the soft `RLIMIT_NOFILE` at startup and fitting
//! the connection caps inside what remains.
//!
//! WHY: every admitted connection holds one descriptor, and both connection caps
//! ([`Limits::global_cap`](crate::tls::listener::Limits::global_cap) before hand-off,
//! [`ConnLimits::max_live`](crate::http::conn_serve::ConnLimits::max_live) after it) are chosen
//! bounds, not measured capacities. A soft limit below their sum (the common Unix default is 1024)
//! lets a flood exhaust descriptors, after which `accept` fails and the server cannot open its
//! database, assets or outbound fetches either. The budget makes the caps structurally unable to
//! promise more descriptors than the process may hold.
//!
//! INVARIANTS:
//! 1. Both caps derive from ONE budget, the effective soft limit read back with `getrlimit` AFTER
//!    the raise attempt minus [`descriptors_reserved`], never from the value the raise asked for:
//!    a failed or partial raise must constrain the caps the same way a limit that was never
//!    raisable does.
//! 2. A descriptor is held by a connection either before hand-off or after it, never both, but a
//!    connection is admitted to the next stage before the previous one releases, so the caps are
//!    fitted to their SUM. The hand-off queue ([`HANDOFF_QUEUE_CAP`]) holds descriptors that belong
//!    to neither cap, so its capacity is subtracted too, as is [`STREAMED_FILE_PERMITS`], the size of
//!    the semaphore every streamed asset file takes a permit from, which bounds the asset files held
//!    open for stalled peers.
//! 3. The raise and the fit read ONE definition of the need: [`descriptors_needed`] is
//!    [`descriptors_reserved`] plus the two caps, and `fit_to_descriptor_budget` subtracts
//!    [`descriptors_reserved`] from the soft limit, so a change to a cap or a reserved term moves
//!    both.
//! 4. The platform ceiling is queried at runtime (macOS `kern.maxfilesperproc`), never a
//!    remembered literal, because `setrlimit` on macOS rejects a soft limit above it even when the
//!    hard limit is unlimited.
//! 5. A platform without `RLIMIT_NOFILE` reports no limit and the caps are not clamped; no ceiling
//!    is claimed for it.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use crate::http::conn_serve::ConnLimits;
use crate::tls::listener::{Limits, HANDOFF_QUEUE_CAP};

/// Descriptors held back from the connection caps for everything that is not a client
/// connection: the database and its write-ahead log, asset and backup files, the log file, the
/// listeners, outbound fetches (link previews, module downloads) and the runtime's own handles.
/// Chosen, not measured: generous against a table's handful of concurrent file operations, and
/// small against any raised limit.
pub const FD_RESERVE: u64 = 512;

/// The most asset files the server holds open at once while streaming them to peers. A streamed
/// response keeps its descriptor for as long as its peer stalls, so the descriptor budget owns the
/// bound: [`descriptors_reserved`] subtracts it, and
/// `http::assets::permits::StreamedFilePermits::production` creates the semaphore of exactly this
/// size that `AppState` holds. `http::assets::serve`, `http::assets::mutate::original` and
/// `http::world_bundle::export_world` each take a permit before opening a file and release it when
/// the file is closed; a request that cannot get one within `permits::PERMIT_WAIT` is answered
/// 503 with `Retry-After`. Chosen against a measured workload: a debug-profile gauge of the pool's
/// peak recorded 3 to 4 permits held while Chromium loaded a scene of 150 distinct token images
/// over HTTP/1.1 and over HTTP/2, so 256 is far above one scene load, which is the bursty
/// workload; it is also a table's players times a few concurrent media or download requests, and
/// small against the connection caps.
pub const STREAMED_FILE_PERMITS: usize = 256;

/// Floor of [`Limits::global_cap`] after clamping. A server that admits nothing is not serving;
/// when the descriptor limit is at or below [`descriptors_reserved`] the floors win over the budget and the
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
/// use shadowcat::fd_budget::{descriptors_needed, raise_nofile_limit};
/// use shadowcat::http::conn_serve::ConnLimits;
/// use shadowcat::tls::listener::Limits;
///
/// let limit = raise_nofile_limit(descriptors_needed(&Limits::default(), &ConnLimits::default()));
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
    /// `soft - descriptors_reserved()`, saturating at zero.
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

/// The descriptors the connection caps cannot use: [`FD_RESERVE`], the hand-off queue's
/// [`HANDOFF_QUEUE_CAP`] and [`STREAMED_FILE_PERMITS`]. The single definition behind both
/// [`descriptors_needed`] and [`fit_to_descriptor_budget`].
///
/// # Examples
///
/// ```
/// use shadowcat::fd_budget::{descriptors_reserved, FD_RESERVE, STREAMED_FILE_PERMITS};
/// use shadowcat::tls::listener::HANDOFF_QUEUE_CAP;
///
/// assert_eq!(
///     descriptors_reserved(),
///     FD_RESERVE + HANDOFF_QUEUE_CAP as u64 + STREAMED_FILE_PERMITS as u64
/// );
/// ```
pub fn descriptors_reserved() -> u64 {
    FD_RESERVE + HANDOFF_QUEUE_CAP as u64 + STREAMED_FILE_PERMITS as u64
}

/// The soft `RLIMIT_NOFILE` the process needs to back the caps `pre` and `live` AND everything
/// [`descriptors_reserved`] counts: what startup asks the raise for, and the smallest soft limit
/// at which [`fit_to_descriptor_budget`] changes nothing.
///
/// # Examples
///
/// ```
/// use shadowcat::fd_budget::{descriptors_needed, fit_to_descriptor_budget};
/// use shadowcat::http::conn_serve::ConnLimits;
/// use shadowcat::tls::listener::Limits;
///
/// let (pre, live) = (Limits::default(), ConnLimits::default());
/// let needed = descriptors_needed(&pre, &live);
/// assert!(fit_to_descriptor_budget(Some(needed), pre, live).2.is_none());
/// assert!(fit_to_descriptor_budget(Some(needed - 1), pre, live).2.is_some());
/// ```
pub fn descriptors_needed(pre: &Limits, live: &ConnLimits) -> u64 {
    pre.global_cap as u64 + live.max_live as u64 + descriptors_reserved()
}

/// Fits both connection caps inside `soft - descriptors_reserved()` descriptors, keeping their ratio, and
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
/// use shadowcat::fd_budget::{descriptors_reserved, fit_to_descriptor_budget};
/// use shadowcat::http::conn_serve::ConnLimits;
/// use shadowcat::tls::listener::Limits;
///
/// let (pre, live, clamp) =
///     fit_to_descriptor_budget(Some(1024), Limits::default(), ConnLimits::default());
/// assert!(clamp.is_some());
/// assert!(pre.global_cap as u64 + live.max_live as u64 <= 1024 - descriptors_reserved());
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
    let budget = soft.saturating_sub(descriptors_reserved());
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

/// The startup sequence every serving binary runs: raise the soft `RLIMIT_NOFILE` through `raise`
/// to [`descriptors_needed`] for `pre` and `live`, then fit both caps to the limit `raise` reports
/// back ([`fit_to_descriptor_budget`]), warning once when the fit reduced them. `raise` receives the
/// need and returns the limit as it stands afterwards; production passes [`raise_nofile_limit`]
/// through [`raise_and_fit`], a test passes a stand-in built on [`raise_with`].
///
/// # Examples
///
/// ```
/// use shadowcat::fd_budget::{descriptors_needed, raise_and_fit_with, raise_with};
/// use shadowcat::http::conn_serve::ConnLimits;
/// use shadowcat::tls::listener::Limits;
///
/// let (pre, live) = (Limits::default(), ConnLimits::default());
/// let needed = descriptors_needed(&pre, &live);
/// let (fitted_pre, fitted_live) = raise_and_fit_with(
///     |need| raise_with(|| Some((256, 1 << 20)), |_, _| Ok(()), None, need, u64::MAX),
///     pre,
///     live,
/// );
/// // The stand-in reads back 256 after the "raise", so the caps shrink to fit it.
/// assert!(fitted_pre.global_cap < pre.global_cap);
/// assert!(fitted_live.max_live < live.max_live);
/// assert!(needed > 256);
/// ```
pub fn raise_and_fit_with(
    raise: impl FnOnce(u64) -> NofileLimit,
    pre: Limits,
    live: ConnLimits,
) -> (Limits, ConnLimits) {
    let nofile = raise(descriptors_needed(&pre, &live));
    let (pre, live, clamp) = fit_to_descriptor_budget(nofile.soft, pre, live);
    if let Some(clamp) = clamp {
        tracing::warn!(
            soft_limit = clamp.soft,
            budget = clamp.budget,
            pre_handoff_cap = clamp.pre_handoff_after,
            live_cap = clamp.live_after,
            floors_exceed_budget = clamp.floors_exceed_budget,
            "the open-file limit cannot back the default connection caps plus the descriptors \
             reserved for the hand-off queue, streamed asset files and everything else; the caps \
             were reduced to fit. Raise the limit (ulimit -n / LimitNOFILE) to restore them"
        );
    }
    (pre, live)
}

/// [`raise_and_fit_with`] over the process's real limit ([`raise_nofile_limit`]): the one call
/// `main` and the `test_server` binary make before binding, so the browser-suite server runs under
/// the same descriptor budget as production.
///
/// # Examples
///
/// ```
/// use shadowcat::fd_budget::raise_and_fit;
/// use shadowcat::http::conn_serve::ConnLimits;
/// use shadowcat::tls::listener::Limits;
///
/// let (pre, live) = raise_and_fit(Limits::default(), ConnLimits::default());
/// assert!(pre.global_cap > 0 && live.max_live > 0);
/// ```
pub fn raise_and_fit(pre: Limits, live: ConnLimits) -> (Limits, ConnLimits) {
    raise_and_fit_with(raise_nofile_limit, pre, live)
}

/// Raises the soft `RLIMIT_NOFILE` to `needed` descriptors (see [`descriptors_needed`]), bounded by
/// the hard limit and the platform ceiling, and returns the soft limit read back afterwards. A
/// limit already at or above `needed` is left alone. A refused raise is logged at `warn` and the
/// current limit is used.
///
/// # Examples
///
/// ```
/// use shadowcat::fd_budget::{descriptors_needed, raise_nofile_limit};
/// use shadowcat::http::conn_serve::ConnLimits;
/// use shadowcat::tls::listener::Limits;
///
/// let needed = descriptors_needed(&Limits::default(), &ConnLimits::default());
/// let limit = raise_nofile_limit(needed);
/// assert!(limit.soft.map_or(true, |soft| soft > 0));
/// ```
#[cfg(unix)]
pub fn raise_nofile_limit(needed: u64) -> NofileLimit {
    raise_with(
        read_nofile,
        write_nofile_soft,
        platform_ceiling(),
        needed,
        widen(libc::RLIM_INFINITY),
    )
}

/// On a platform without `RLIMIT_NOFILE` there is no limit to raise and none to budget against:
/// the caps are not clamped, and no descriptor ceiling is claimed for the platform. `_needed` is
/// the parameter the Unix variant raises to.
///
/// # Examples
///
/// ```
/// use shadowcat::fd_budget::{descriptors_needed, raise_nofile_limit};
/// use shadowcat::http::conn_serve::ConnLimits;
/// use shadowcat::tls::listener::Limits;
///
/// let needed = descriptors_needed(&Limits::default(), &ConnLimits::default());
/// let limit = raise_nofile_limit(needed);
/// assert!(limit.soft.map_or(true, |soft| soft > 0));
/// ```
#[cfg(not(unix))]
pub fn raise_nofile_limit(_needed: u64) -> NofileLimit {
    NofileLimit {
        soft: None,
        before: None,
        problem: None,
    }
}

/// The raise decision, independent of the platform so every OS's CI leg runs it: `read` returns
/// `(soft, hard)` or `None` when the limit cannot be read, `write(soft, hard)` sets the soft limit
/// while keeping `hard`, `ceiling` is the platform ceiling and `unlimited` the value `read` uses
/// for "no limit". The soft limit is raised to [`raise_target`] only when it is below `needed`, and
/// the result is read back through `read` again, so a refused or partial raise is reported as the
/// limit the process actually has.
///
/// # Examples
///
/// ```
/// use shadowcat::fd_budget::raise_with;
///
/// let limit = raise_with(|| Some((1024, 4096)), |_, _| Ok(()), None, 2048, u64::MAX);
/// assert_eq!(limit.before, Some(1024));
/// assert!(limit.problem.is_none());
/// ```
pub fn raise_with(
    read: impl Fn() -> Option<(u64, u64)>,
    write: impl Fn(u64, u64) -> std::io::Result<()>,
    ceiling: Option<u64>,
    needed: u64,
    unlimited: u64,
) -> NofileLimit {
    let bounded = |limit: u64| (limit != unlimited).then_some(limit);
    let describe = |limit: u64| match bounded(limit) {
        Some(n) => n.to_string(),
        None => "unlimited".to_string(),
    };
    let Some((before_soft, hard)) = read() else {
        let problem = format!(
            "reading the open-file limit failed: {}",
            std::io::Error::last_os_error()
        );
        tracing::warn!("{problem}; the connection caps are not fitted to a descriptor limit");
        return NofileLimit {
            soft: None,
            before: None,
            problem: Some(problem),
        };
    };
    let target = raise_target(before_soft, hard, ceiling, needed);
    let mut problem = None;
    if target > before_soft {
        if let Err(e) = write(target, hard) {
            let message = format!(
                "raising the open-file limit from {} to {target} failed: {e}",
                describe(before_soft)
            );
            tracing::warn!("{message}; continuing with the current limit");
            problem = Some(message);
        }
    }
    let after = read().map(|(soft, _)| soft);
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

/// The soft limit a raise should ask for: `needed`, bounded by the hard limit and the platform
/// `ceiling`, and never below the current soft limit. A limit that already covers `needed` is not
/// raised, and an unlimited hard limit does not make the target unlimited.
///
/// # Examples
///
/// ```
/// use shadowcat::fd_budget::raise_target;
///
/// assert_eq!(raise_target(1024, 65_536, None, 3_000), 3_000);
/// assert_eq!(raise_target(1024, 2_048, None, 3_000), 2_048);
/// assert_eq!(raise_target(1024, u64::MAX, Some(2_500), 3_000), 2_500);
/// assert_eq!(raise_target(20_000, 65_536, Some(10_240), 3_000), 20_000);
/// ```
pub fn raise_target(soft: u64, hard: u64, ceiling: Option<u64>, needed: u64) -> u64 {
    let bound = ceiling.map_or(hard, |c| hard.min(c));
    needed.min(bound).max(soft)
}

/// Widens a platform `rlim_t` (32 or 64 bits depending on the target) to `u64` without a cast
/// that is redundant on the targets where it is already `u64`.
#[cfg(unix)]
fn widen<T: Into<u64>>(value: T) -> u64 {
    value.into()
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
/// (`setrlimit(2)` on Darwin, EINVAL).
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

/// The highest soft limit `setrlimit` accepts on this platform when it is lower than the hard
/// limit can say; this platform has no such ceiling to claim. A refused raise is handled by
/// [`raise_with`], not prevented here.
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
