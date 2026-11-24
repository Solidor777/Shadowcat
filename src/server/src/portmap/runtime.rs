//! The port-mapping runtime: the single enable decision, the task that holds a mapping, and the
//! status the admin route reports.
//!
//! INVARIANT: at most one mapping is held. The mapping lives in `Shared::slot`, written with no
//! await between the router's grant and the write, so stopping the task by aborting it at any
//! await leaves the slot equal to what the router holds, and [`PortMapRuntime::shutdown`] deletes
//! exactly that.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::task::JoinHandle;

use super::mapping::{self, ActiveMapping, Failure, Targets};
use super::status::{is_public_address, PortMapState, PortMapStatus};
use crate::config::Config;
use crate::data::sqlite::SqliteRepository;

/// The `settings` key holding the admin's stored toggle (`"true"` or `"false"`).
///
/// # Examples
///
/// ```
/// assert_eq!(shadowcat::portmap::runtime::STORED_TOGGLE_KEY, "port_forward_enabled");
/// ```
pub const STORED_TOGGLE_KEY: &str = "port_forward_enabled";

/// Whether port forwarding is on: the `port_forward` configuration key when set, else the stored
/// toggle. The runtime, the admin route and the status all call this one function.
///
/// # Examples
///
/// ```
/// use shadowcat::config::Config;
/// use shadowcat::portmap::runtime::effective_enabled;
/// let locked_off = Config { port_forward: Some(false), ..Config::default() };
/// assert!(!effective_enabled(&locked_off, true));
/// assert!(effective_enabled(&Config::default(), true));
/// ```
pub fn effective_enabled(config: &Config, stored: bool) -> bool {
    config.port_forward.unwrap_or(stored)
}

/// Reads the stored toggle; a missing row, an unreadable one or any other value is `false`.
///
/// # Examples
///
/// ```
/// # #[tokio::main] async fn main() {
/// use shadowcat::data::sqlite::SqliteRepository;
/// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
/// assert!(!shadowcat::portmap::runtime::read_stored_toggle(&repo).await);
/// # }
/// ```
pub async fn read_stored_toggle(repo: &SqliteRepository) -> bool {
    match repo.get_setting(STORED_TOGGLE_KEY).await {
        Ok(value) => value.as_deref() == Some("true"),
        Err(error) => {
            tracing::warn!(%error, "port forwarding toggle unreadable; treating it as off");
            false
        }
    }
}

/// Persists the toggle.
///
/// # Errors
///
/// The repository's error.
///
/// # Examples
///
/// ```
/// # #[tokio::main] async fn main() {
/// use shadowcat::data::sqlite::SqliteRepository;
/// use shadowcat::portmap::runtime::{read_stored_toggle, write_stored_toggle};
/// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
/// write_stored_toggle(&repo, true).await.unwrap();
/// assert!(read_stored_toggle(&repo).await);
/// # }
/// ```
pub async fn write_stored_toggle(
    repo: &SqliteRepository,
    enabled: bool,
) -> Result<(), crate::data::DataError> {
    repo.set_setting(STORED_TOGGLE_KEY, if enabled { "true" } else { "false" })
        .await
}

/// Locks `mutex`, ignoring poisoning: the guarded values are plain data a panic cannot leave
/// half-written.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the task is doing when it holds no mapping.
#[derive(Debug, Clone)]
struct Live {
    /// Effective enable decision at the last reconcile.
    enabled: bool,
    /// State reported when no mapping is held.
    state: PortMapState,
    /// Why the last attempt failed, or why the bind is unreachable.
    failure: Option<Failure>,
}

/// State shared between the runtime handle and its task.
struct Shared {
    /// The configuration the enable decision reads.
    config: Arc<Config>,
    /// Where the protocols talk.
    targets: Targets,
    /// The listener's address, set once the server is bound.
    listen: Mutex<Option<SocketAddr>>,
    /// Status when no mapping is held.
    live: Mutex<Live>,
    /// The mapping the router holds for this server.
    slot: Mutex<Option<ActiveMapping>>,
    /// The mapping task. The lock serializes every start and stop.
    task: tokio::sync::Mutex<Option<JoinHandle<()>>>,
}

/// Holds the router mapping for the server's listening port.
///
/// # Examples
///
/// ```
/// use shadowcat::portmap::runtime::PortMapRuntime;
/// let runtime = PortMapRuntime::disabled();
/// assert!(!runtime.status().enabled);
/// ```
pub struct PortMapRuntime {
    /// The shared state.
    shared: Arc<Shared>,
}

impl PortMapRuntime {
    /// A runtime that talks to the real network and obeys `config`.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use shadowcat::config::Config;
    /// let runtime = shadowcat::portmap::runtime::PortMapRuntime::new(Arc::new(Config::default()));
    /// assert!(!runtime.locked_by_config());
    /// ```
    pub fn new(config: Arc<Config>) -> Arc<Self> {
        Self::with_targets(config, Targets::production())
    }

    /// A runtime with the default configuration that never starts until activated and enabled.
    ///
    /// # Examples
    ///
    /// ```
    /// let runtime = shadowcat::portmap::runtime::PortMapRuntime::disabled();
    /// assert!(!runtime.status().enabled);
    /// ```
    pub fn disabled() -> Arc<Self> {
        Self::new(Arc::new(Config::default()))
    }

    /// A runtime pointed at `targets`.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use shadowcat::config::Config;
    /// use shadowcat::portmap::mapping::Targets;
    /// use shadowcat::portmap::runtime::PortMapRuntime;
    /// let runtime = PortMapRuntime::with_targets(Arc::new(Config::default()), Targets::production());
    /// assert!(!runtime.status().enabled);
    /// ```
    pub fn with_targets(config: Arc<Config>, targets: Targets) -> Arc<Self> {
        Arc::new(Self {
            shared: Arc::new(Shared {
                config,
                targets,
                listen: Mutex::new(None),
                live: Mutex::new(Live {
                    enabled: false,
                    state: PortMapState::Disabled,
                    failure: None,
                }),
                slot: Mutex::new(None),
                task: tokio::sync::Mutex::new(None),
            }),
        })
    }

    /// Whether the `port_forward` configuration key fixes the toggle.
    ///
    /// # Examples
    ///
    /// ```
    /// assert!(!shadowcat::portmap::runtime::PortMapRuntime::disabled().locked_by_config());
    /// ```
    pub fn locked_by_config(&self) -> bool {
        self.shared.config.port_forward.is_some()
    }

    /// Records the bound listener and applies the enable decision. Called once after the listener
    /// is bound, so accepting connections never waits on discovery.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # #[tokio::main] async fn main() {
    /// let runtime = shadowcat::portmap::runtime::PortMapRuntime::disabled();
    /// runtime.activate("0.0.0.0:30000".parse().unwrap(), false).await;
    /// # }
    /// ```
    pub async fn activate(&self, listen: SocketAddr, stored_enabled: bool) {
        *lock(&self.shared.listen) = Some(listen);
        self.apply(stored_enabled).await;
    }

    /// Applies the enable decision for `stored_enabled` and the configuration: starts the task,
    /// or stops it and deletes the mapping.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main] async fn main() {
    /// let runtime = shadowcat::portmap::runtime::PortMapRuntime::disabled();
    /// runtime.apply(false).await;
    /// assert!(!runtime.status().enabled);
    /// # }
    /// ```
    pub async fn apply(&self, stored_enabled: bool) {
        let enabled = effective_enabled(&self.shared.config, stored_enabled);
        let mut task = self.shared.task.lock().await;
        let listen = *lock(&self.shared.listen);
        let set = |state: PortMapState, failure: Option<Failure>| {
            *lock(&self.shared.live) = Live {
                enabled,
                state,
                failure,
            };
        };
        if !enabled {
            stop(&self.shared, &mut task).await;
            set(PortMapState::Disabled, None);
            return;
        }
        let Some(listen) = listen else {
            stop(&self.shared, &mut task).await;
            set(
                PortMapState::Failed,
                Some(Failure::new(
                    "not_listening",
                    "the server is not listening yet",
                )),
            );
            return;
        };
        if listen.ip().is_loopback() {
            stop(&self.shared, &mut task).await;
            set(
                PortMapState::UnreachableBind,
                Some(Failure::new(
                    "unreachable_bind",
                    "the server listens on a loopback address, which a router cannot forward to",
                )),
            );
            return;
        }
        if task.as_ref().is_some_and(|handle| !handle.is_finished()) {
            lock(&self.shared.live).enabled = true;
            return;
        }
        set(PortMapState::Discovering, None);
        *task = Some(tokio::spawn(run(Arc::clone(&self.shared), listen.port())));
    }

    /// The current status.
    ///
    /// # Examples
    ///
    /// ```
    /// let status = shadowcat::portmap::runtime::PortMapRuntime::disabled().status();
    /// assert!(!status.locked_by_config);
    /// ```
    pub fn status(&self) -> PortMapStatus {
        let live = lock(&self.shared.live).clone();
        let locked_by_config = self.locked_by_config();
        let slot = lock(&self.shared.slot).clone();
        match slot {
            Some(mapping) if live.enabled => {
                let public = mapping.external_address.is_none_or(is_public_address);
                PortMapStatus {
                    enabled: true,
                    locked_by_config,
                    state: if public {
                        PortMapState::Mapped
                    } else {
                        PortMapState::MappedNotPublic
                    },
                    protocol: Some(mapping.protocol),
                    external_address: mapping.external_address.map(|ip| ip.to_string()),
                    external_port: Some(mapping.external_port),
                    lease_expires_in_secs: mapping.lease_expires_in_secs(),
                    reason: (!public).then(|| "mapped_not_public".to_owned()),
                    detail: (!public).then(|| {
                        "the router's external address is not publicly routable; players on the internet cannot reach this server".to_owned()
                    }),
                }
            }
            _ => PortMapStatus {
                enabled: live.enabled,
                locked_by_config,
                state: live.state,
                protocol: None,
                external_address: None,
                external_port: None,
                lease_expires_in_secs: None,
                reason: live.failure.as_ref().map(|f| f.reason.to_owned()),
                detail: live.failure.map(|f| f.detail),
            },
        }
    }

    /// The router's external address while a mapping is held AND that address is publicly
    /// routable: the address a host hands to players on the internet. `None` otherwise (nothing
    /// mapped, the router did not report one, or it is not public). It is the one reader the
    /// certificate shown to public peers consults ([`crate::tls::TlsRuntime::set_external_source`]).
    ///
    /// # Examples
    ///
    /// ```
    /// assert!(shadowcat::portmap::runtime::PortMapRuntime::disabled().public_address().is_none());
    /// ```
    pub fn public_address(&self) -> Option<std::net::IpAddr> {
        let status = self.status();
        if status.state != PortMapState::Mapped {
            return None;
        }
        status
            .external_address
            .and_then(|text| text.parse().ok())
            .filter(|ip| is_public_address(*ip))
    }

    /// Stops the task and deletes the mapping within the deletion deadline. Called after the HTTP
    /// drain. A mapping that cannot be deleted expires with its lease.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[tokio::main] async fn main() {
    /// shadowcat::portmap::runtime::PortMapRuntime::disabled().shutdown().await;
    /// # }
    /// ```
    pub async fn shutdown(&self) {
        let mut task = self.shared.task.lock().await;
        stop(&self.shared, &mut task).await;
    }
}

/// Aborts the task, waits for it to end, then deletes the recorded mapping.
async fn stop(shared: &Shared, task: &mut Option<JoinHandle<()>>) {
    if let Some(handle) = task.take() {
        handle.abort();
        let _ = handle.await;
    }
    let held = lock(&shared.slot).take();
    if let Some(mapping) = held {
        mapping::delete(&shared.targets, &mapping).await;
    }
}

/// Logs the public address once a mapping is confirmed public.
fn log_reachable(mapping: &ActiveMapping) {
    if let Some(ip) = mapping.external_address.filter(|ip| is_public_address(*ip)) {
        let addr = SocketAddr::new(ip, mapping.external_port);
        tracing::info!("reachable at http://{addr}");
    }
}

/// The mapping task: discover, hold, renew at half the lease, and retry with back-off.
async fn run(shared: Arc<Shared>, port: u16) {
    let targets = &shared.targets;
    let mut backoff = targets.backoff_initial;
    loop {
        set_state(&shared, PortMapState::Discovering, None);
        match mapping::acquire(targets, port).await {
            Ok(mapping) => {
                tracing::info!(protocol = ?mapping.protocol, external_port = mapping.external_port, "router port mapping granted");
                log_reachable(&mapping);
                *lock(&shared.slot) = Some(mapping);
                backoff = targets.backoff_initial;
                hold(&shared).await;
            }
            Err(failure) => {
                tracing::warn!(reason = failure.reason, detail = %failure.detail, "router port mapping failed");
                set_state(&shared, PortMapState::Failed, Some(failure));
            }
        }
        tokio::time::sleep(backoff).await;
        backoff = backoff
            .checked_mul(2)
            .unwrap_or(targets.backoff_max)
            .min(targets.backoff_max);
    }
}

/// Renews the recorded mapping at half its lease until a renewal fails, then removes it and
/// records the failure.
async fn hold(shared: &Shared) {
    loop {
        let Some(current) = lock(&shared.slot).clone() else {
            return;
        };
        tokio::time::sleep(current.renew_after()).await;
        match mapping::renew(&shared.targets, &current).await {
            Ok(renewed) => {
                if renewed.external_port != current.external_port
                    || renewed.external_address != current.external_address
                {
                    log_reachable(&renewed);
                }
                *lock(&shared.slot) = Some(renewed);
            }
            Err(failure) => {
                tracing::warn!(reason = failure.reason, detail = %failure.detail, "router port mapping renewal failed");
                let held = lock(&shared.slot).take();
                if let Some(old) = held {
                    mapping::delete(&shared.targets, &old).await;
                }
                set_state(shared, PortMapState::Failed, Some(failure));
                return;
            }
        }
    }
}

/// Records the state reported while no mapping is held.
fn set_state(shared: &Shared, state: PortMapState, failure: Option<Failure>) {
    let mut live = lock(&shared.live);
    live.state = state;
    live.failure = failure;
}

#[cfg(test)]
mod tests;
