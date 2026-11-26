//! One router mapping and the protocol ladder that obtains, renews and deletes it.
//!
//! [`acquire`] tries NAT-PMP, then PCP, then UPnP IGD, and returns the first mapping a router
//! grants. The grant is the LAST await on every path, so the caller can record the returned
//! [`ActiveMapping`] with no await point between the router creating the mapping and the record:
//! cancelling the task at any await never leaves an unrecorded mapping behind.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use tokio::time::Instant;

use super::client::{
    natpmp_delete, natpmp_external_address, natpmp_map, pcp_delete, pcp_map, ClientError,
    RetryPolicy,
};
use super::codec::{gateway_restarted, GATEWAY_PORT, PCP_NONCE_LEN};
use super::igd::{self, IgdGateway, REQUESTED_LEASE_SECS};
use super::soap::{is_lan_address, IgdError};
use super::ssdp::{COLLECT_WINDOW, SSDP_GROUP};
use super::status::PortMapProtocol;

/// A failed attempt: a machine key for the admin status and a sentence for the log.
///
/// # Examples
///
/// ```
/// use shadowcat::portmap::mapping::Failure;
/// let failure = Failure::new("no_response", "nothing answered");
/// assert_eq!(failure.reason, "no_response");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// Machine-readable key (`no_gateway`, `no_response`, `refused`, `untrusted_endpoint`,
    /// `io_error`, `igd_error`).
    pub reason: &'static str,
    /// Human-readable sentence.
    pub detail: String,
}

impl Failure {
    /// Builds a failure.
    ///
    /// # Examples
    ///
    /// ```
    /// let failure = shadowcat::portmap::mapping::Failure::new("refused", "no");
    /// assert_eq!(failure.detail, "no");
    /// ```
    pub fn new(reason: &'static str, detail: impl Into<String>) -> Self {
        Self {
            reason,
            detail: detail.into(),
        }
    }
}

impl From<ClientError> for Failure {
    /// Classifies a NAT-PMP or PCP exchange failure.
    fn from(error: ClientError) -> Self {
        let reason = match error {
            ClientError::Timeout => "no_response",
            ClientError::Refused(_) | ClientError::Mismatch => "refused",
            ClientError::Io(_) => "io_error",
        };
        Self::new(reason, error.to_string())
    }
}

impl From<IgdError> for Failure {
    /// Classifies a UPnP failure.
    fn from(error: IgdError) -> Self {
        let reason = match error {
            IgdError::NoResponse | IgdError::Timeout => "no_response",
            IgdError::Untrusted => "untrusted_endpoint",
            IgdError::Fault(_) | IgdError::Status(_) => "refused",
            IgdError::Io(_) => "io_error",
            _ => "igd_error",
        };
        Self::new(reason, error.to_string())
    }
}

/// Where and how long the protocols talk. [`Targets::production`] is the live configuration; the
/// fields are public so a test can point every protocol at a loopback fake.
///
/// # Examples
///
/// ```
/// use shadowcat::portmap::mapping::Targets;
/// let targets = Targets::production();
/// assert!(targets.gateway.is_none());
/// ```
#[derive(Debug, Clone)]
pub struct Targets {
    /// The NAT-PMP/PCP gateway; `None` asks the platform for the default gateway.
    pub gateway: Option<SocketAddr>,
    /// Where SSDP searches go.
    pub ssdp: SocketAddr,
    /// How long SSDP replies are collected.
    pub ssdp_window: Duration,
    /// Which addresses count as a LAN for UPnP endpoint trust.
    pub accept: fn(IpAddr) -> bool,
    /// Retransmission schedule of NAT-PMP and PCP requests.
    pub retry: RetryPolicy,
    /// First wait after a failed attempt; doubles up to `backoff_max`.
    pub backoff_initial: Duration,
    /// Longest wait between attempts.
    pub backoff_max: Duration,
    /// Total time one mapping deletion may take.
    pub delete_deadline: Duration,
}

impl Targets {
    /// The live configuration: default gateway, the SSDP multicast group, LAN-only UPnP endpoints,
    /// a 60 s back-off doubling to 15 min, and a 3 s deletion deadline.
    ///
    /// # Examples
    ///
    /// ```
    /// let targets = shadowcat::portmap::mapping::Targets::production();
    /// assert_eq!(targets.backoff_max.as_secs(), 900);
    /// ```
    pub fn production() -> Self {
        Self {
            gateway: None,
            ssdp: SSDP_GROUP,
            ssdp_window: COLLECT_WINDOW,
            accept: is_lan_address,
            retry: RetryPolicy::PRODUCTION,
            backoff_initial: Duration::from_secs(60),
            backoff_max: Duration::from_secs(900),
            delete_deadline: Duration::from_secs(3),
        }
    }
}

/// How an [`ActiveMapping`] is renewed and deleted.
#[derive(Debug, Clone)]
enum Handle {
    /// A NAT-PMP mapping on this gateway.
    NatPmp(SocketAddr),
    /// A PCP mapping on this gateway, identified by its nonce.
    Pcp(SocketAddr, [u8; PCP_NONCE_LEN]),
    /// A UPnP mapping on this gateway.
    Igd(IgdGateway),
}

/// The one mapping the server holds on a router.
///
/// # Examples
///
/// ```no_run
/// # async fn demo() {
/// use shadowcat::portmap::mapping::{acquire, Targets};
/// if let Ok(mapping) = acquire(&Targets::production(), 30000).await {
///     println!("external port {}", mapping.external_port);
/// }
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct ActiveMapping {
    /// The protocol that holds the mapping.
    pub protocol: PortMapProtocol,
    /// The local port mapped.
    pub internal_port: u16,
    /// The external port the router granted.
    pub external_port: u16,
    /// The router's external address, when it reported one.
    pub external_address: Option<IpAddr>,
    /// Granted lease in seconds; 0 marks a permanent mapping.
    pub lease_secs: u32,
    /// When the router granted it.
    pub granted_at: Instant,
    /// The gateway's seconds-since-start at the grant (NAT-PMP and PCP).
    epoch: Option<u32>,
    /// How to renew and delete it.
    handle: Handle,
}

impl ActiveMapping {
    /// How long to wait before renewing: half the lease, at least one second. A permanent mapping
    /// is re-checked at half the requested lease so a router that lost it is noticed.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn demo() {
    /// use shadowcat::portmap::mapping::{acquire, Targets};
    /// if let Ok(mapping) = acquire(&Targets::production(), 30000).await {
    ///     let wait = mapping.renew_after(); // 1800 s for a 3600 s lease
    ///     println!("renew in {wait:?}");
    /// }
    /// # }
    /// ```
    pub fn renew_after(&self) -> Duration {
        let lease = if self.lease_secs == 0 {
            REQUESTED_LEASE_SECS
        } else {
            self.lease_secs
        };
        Duration::from_secs(u64::from(lease / 2).max(1))
    }

    /// Seconds until the lease ends; `None` for a permanent mapping.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn demo() {
    /// use shadowcat::portmap::mapping::{acquire, Targets};
    /// if let Ok(mapping) = acquire(&Targets::production(), 30000).await {
    ///     println!("{:?} s left", mapping.lease_expires_in_secs());
    /// }
    /// # }
    /// ```
    pub fn lease_expires_in_secs(&self) -> Option<u32> {
        if self.lease_secs == 0 {
            return None;
        }
        let elapsed = u32::try_from(self.granted_at.elapsed().as_secs()).unwrap_or(u32::MAX);
        Some(self.lease_secs.saturating_sub(elapsed))
    }
}

/// The default gateway's NAT-PMP/PCP address.
///
/// Only an IPv4 gateway is used: NAT-PMP is IPv4-only and the IPv4 PCP path covers the routers
/// that speak PCP. The lookup reads the routing table, which can block, so it runs on the blocking
/// pool.
async fn resolve_gateway(targets: &Targets) -> Option<SocketAddr> {
    if let Some(addr) = targets.gateway {
        return Some(addr);
    }
    let device = tokio::task::spawn_blocking(netdev::get_default_gateway)
        .await
        .ok()?
        .ok()?;
    let ip = device.ipv4.first().copied()?;
    Some(SocketAddr::new(IpAddr::V4(ip), GATEWAY_PORT))
}

/// A fresh random PCP nonce.
fn new_nonce() -> [u8; PCP_NONCE_LEN] {
    let bytes = uuid::Uuid::new_v4().into_bytes();
    bytes
        .first_chunk::<PCP_NONCE_LEN>()
        .copied()
        .unwrap_or([0; PCP_NONCE_LEN])
}

/// One NAT-PMP request cycle: the external address, then the mapping. The mapping is last so the
/// grant is the final await.
async fn natpmp_attempt(
    gateway: SocketAddr,
    internal_port: u16,
    suggested: u16,
    retry: RetryPolicy,
) -> Result<ActiveMapping, ClientError> {
    let (address, _) = natpmp_external_address(gateway, retry).await?;
    let grant = natpmp_map(
        gateway,
        internal_port,
        suggested,
        REQUESTED_LEASE_SECS,
        retry,
    )
    .await?;
    if grant.lifetime_secs == 0 {
        return Err(ClientError::Mismatch);
    }
    Ok(ActiveMapping {
        protocol: PortMapProtocol::NatPmp,
        internal_port,
        external_port: grant.external_port,
        external_address: Some(IpAddr::V4(address)),
        lease_secs: grant.lifetime_secs,
        granted_at: Instant::now(),
        epoch: Some(grant.epoch),
        handle: Handle::NatPmp(gateway),
    })
}

/// One PCP MAP cycle with `nonce`.
async fn pcp_attempt(
    gateway: SocketAddr,
    nonce: [u8; PCP_NONCE_LEN],
    internal_port: u16,
    suggested: u16,
    retry: RetryPolicy,
) -> Result<ActiveMapping, ClientError> {
    let grant = pcp_map(
        gateway,
        nonce,
        internal_port,
        suggested,
        REQUESTED_LEASE_SECS,
        retry,
    )
    .await?;
    if grant.lifetime_secs == 0 {
        return Err(ClientError::Mismatch);
    }
    Ok(ActiveMapping {
        protocol: PortMapProtocol::Pcp,
        internal_port,
        external_port: grant.external_port,
        external_address: Some(grant.external_address),
        lease_secs: grant.lifetime_secs,
        granted_at: Instant::now(),
        epoch: Some(grant.epoch),
        handle: Handle::Pcp(gateway, nonce),
    })
}

/// One UPnP cycle on `gateway`: the external address (best effort), then the mapping.
async fn igd_attempt(
    gateway: &IgdGateway,
    internal_port: u16,
    suggested: u16,
) -> Result<ActiveMapping, IgdError> {
    let address = gateway.external_address().await.ok();
    let mapping = gateway.add_mapping(internal_port, suggested).await?;
    Ok(ActiveMapping {
        protocol: PortMapProtocol::Upnp,
        internal_port,
        external_port: mapping.external_port,
        external_address: address,
        lease_secs: mapping.lease_secs,
        granted_at: Instant::now(),
        epoch: None,
        handle: Handle::Igd(gateway.clone()),
    })
}

/// Tries every discovered UPnP gateway until one maps.
async fn igd_acquire(targets: &Targets, internal_port: u16) -> Result<ActiveMapping, Failure> {
    let gateways = igd::discover(targets.ssdp, targets.ssdp_window, targets.accept).await?;
    let mut last = Failure::new("no_response", "no UPnP gateway answered");
    for gateway in &gateways {
        match igd_attempt(gateway, internal_port, internal_port).await {
            Ok(mapping) => return Ok(mapping),
            Err(error) => last = error.into(),
        }
    }
    Err(last)
}

/// Picks the failure worth showing: the first that names a cause, else a missing gateway, else
/// plain silence.
fn summarize(failures: Vec<Failure>) -> Failure {
    let silent = |f: &Failure| f.reason == "no_response" || f.reason == "no_gateway";
    if let Some(specific) = failures.iter().find(|f| !silent(f)) {
        return specific.clone();
    }
    if let Some(missing) = failures.iter().find(|f| f.reason == "no_gateway") {
        return missing.clone();
    }
    Failure::new(
        "no_response",
        "no router answered NAT-PMP, PCP or UPnP".to_owned(),
    )
}

/// Obtains a mapping for `internal_port`: NAT-PMP, then PCP, then UPnP IGD.
///
/// # Errors
///
/// The most informative [`Failure`] of the three attempts.
///
/// # Examples
///
/// ```no_run
/// # async fn demo() {
/// use shadowcat::portmap::mapping::{acquire, Targets};
/// let mapping = acquire(&Targets::production(), 30000).await;
/// # let _ = mapping;
/// # }
/// ```
pub async fn acquire(targets: &Targets, internal_port: u16) -> Result<ActiveMapping, Failure> {
    let mut failures = Vec::new();
    match resolve_gateway(targets).await {
        Some(gateway) => {
            match natpmp_attempt(gateway, internal_port, internal_port, targets.retry).await {
                Ok(mapping) => return Ok(mapping),
                Err(error) => failures.push(error.into()),
            }
            match pcp_attempt(
                gateway,
                new_nonce(),
                internal_port,
                internal_port,
                targets.retry,
            )
            .await
            {
                Ok(mapping) => return Ok(mapping),
                Err(error) => failures.push(error.into()),
            }
        }
        None => failures.push(Failure::new(
            "no_gateway",
            "the platform reports no default IPv4 gateway",
        )),
    }
    match igd_acquire(targets, internal_port).await {
        Ok(mapping) => Ok(mapping),
        Err(failure) => {
            failures.push(failure);
            Err(summarize(failures))
        }
    }
}

/// Renews `active` on the protocol and gateway that granted it. The suggested external port is the
/// one already held, so a renewal keeps the port. A NAT-PMP or PCP reply whose epoch shows the
/// gateway restarted is logged; the renewal request itself recreates the mapping.
///
/// # Errors
///
/// The [`Failure`] of the renewal; the caller then drops to discovery.
///
/// # Examples
///
/// ```no_run
/// # async fn demo(active: shadowcat::portmap::mapping::ActiveMapping) {
/// use shadowcat::portmap::mapping::{renew, Targets};
/// let renewed = renew(&Targets::production(), &active).await;
/// # let _ = renewed;
/// # }
/// ```
pub async fn renew(targets: &Targets, active: &ActiveMapping) -> Result<ActiveMapping, Failure> {
    let renewed = match &active.handle {
        Handle::NatPmp(gateway) => {
            natpmp_attempt(
                *gateway,
                active.internal_port,
                active.external_port,
                targets.retry,
            )
            .await?
        }
        Handle::Pcp(gateway, nonce) => {
            pcp_attempt(
                *gateway,
                *nonce,
                active.internal_port,
                active.external_port,
                targets.retry,
            )
            .await?
        }
        Handle::Igd(gateway) => {
            igd_attempt(gateway, active.internal_port, active.external_port).await?
        }
    };
    if let (Some(before), Some(after)) = (active.epoch, renewed.epoch) {
        if gateway_restarted(before, after, active.granted_at.elapsed().as_secs()) {
            tracing::warn!("the router restarted; its port mapping was recreated");
        }
    }
    Ok(renewed)
}

/// Deletes `active` on its router within the targets' deletion deadline. A failure is logged and
/// otherwise ignored: the lease expires on its own.
///
/// # Examples
///
/// ```no_run
/// # async fn demo(active: shadowcat::portmap::mapping::ActiveMapping) {
/// use shadowcat::portmap::mapping::{delete, Targets};
/// delete(&Targets::production(), &active).await;
/// # }
/// ```
pub async fn delete(targets: &Targets, active: &ActiveMapping) {
    let work = async {
        match &active.handle {
            Handle::NatPmp(gateway) => natpmp_delete(*gateway, active.internal_port, targets.retry)
                .await
                .map_err(Failure::from),
            Handle::Pcp(gateway, nonce) => {
                pcp_delete(*gateway, *nonce, active.internal_port, targets.retry)
                    .await
                    .map_err(Failure::from)
            }
            Handle::Igd(gateway) => gateway
                .delete_mapping(active.external_port)
                .await
                .map_err(Failure::from),
        }
    };
    match tokio::time::timeout(targets.delete_deadline, work).await {
        Ok(Ok(())) => tracing::info!("port mapping removed from the router"),
        Ok(Err(failure)) => {
            tracing::warn!(detail = %failure.detail, "port mapping removal failed; the lease will expire")
        }
        Err(_) => tracing::warn!("port mapping removal timed out; the lease will expire"),
    }
}

#[cfg(test)]
mod tests;
