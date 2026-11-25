//! The status the admin route reports and the address classification behind it.

use std::net::IpAddr;

use serde::Serialize;
use ts_rs::TS;

/// Where the port-mapping task stands.
///
/// # Examples
///
/// ```
/// use shadowcat::portmap::status::PortMapState;
/// assert_eq!(serde_json::to_value(PortMapState::MappedNotPublic).unwrap(), "mapped_not_public");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../types/generated/")]
pub enum PortMapState {
    /// The feature is off; nothing is mapped.
    Disabled,
    /// A discovery or mapping attempt is running.
    Discovering,
    /// The router granted a mapping and its external address is publicly routable.
    Mapped,
    /// The router granted a mapping, but its external address is not publicly routable (double
    /// NAT or carrier-grade NAT), so internet players cannot reach it.
    MappedNotPublic,
    /// The last attempt failed; the task retries after a back-off.
    Failed,
    /// The server listens on a loopback address, which a router cannot forward to.
    UnreachableBind,
}

/// The protocol that holds the active mapping.
///
/// # Examples
///
/// ```
/// use shadowcat::portmap::status::PortMapProtocol;
/// assert_eq!(serde_json::to_value(PortMapProtocol::NatPmp).unwrap(), "nat_pmp");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../types/generated/")]
pub enum PortMapProtocol {
    /// NAT-PMP (RFC 6886).
    NatPmp,
    /// Port Control Protocol (RFC 6887).
    Pcp,
    /// UPnP Internet Gateway Device.
    Upnp,
}

/// The admin-only port-forwarding status (`GET /api/admin/portmap`).
///
/// The external address is the host's home address, so this never reaches a player-visible route.
///
/// # Examples
///
/// ```
/// use shadowcat::portmap::status::{PortMapState, PortMapStatus};
/// let status = PortMapStatus::disabled(false);
/// assert_eq!(status.state, PortMapState::Disabled);
/// assert!(status.external_address.is_none());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export, export_to = "../../types/generated/")]
pub struct PortMapStatus {
    /// Whether port forwarding is on after the configuration override and the stored toggle.
    pub enabled: bool,
    /// Whether the `port_forward` configuration key fixes the toggle, so the admin cannot change it.
    pub locked_by_config: bool,
    /// Where the task stands.
    pub state: PortMapState,
    /// The protocol holding the mapping; `None` when nothing is mapped.
    pub protocol: Option<PortMapProtocol>,
    /// The router's external address, when it reported one.
    pub external_address: Option<String>,
    /// The external port the router granted.
    pub external_port: Option<u16>,
    /// Seconds until the granted lease expires; `None` for a permanent mapping or no mapping.
    pub lease_expires_in_secs: Option<u32>,
    /// A machine-readable key for the failure or the unreachable bind.
    pub reason: Option<String>,
    /// A human-readable sentence for `reason`.
    pub detail: Option<String>,
}

impl PortMapStatus {
    /// A status with nothing mapped and the feature off.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::portmap::status::PortMapStatus;
    /// assert!(PortMapStatus::disabled(true).locked_by_config);
    /// ```
    pub fn disabled(locked_by_config: bool) -> Self {
        Self {
            enabled: false,
            locked_by_config,
            state: PortMapState::Disabled,
            protocol: None,
            external_address: None,
            external_port: None,
            lease_expires_in_secs: None,
            reason: None,
            detail: None,
        }
    }
}

#[cfg(test)]
mod tests;

/// True when `ip` is routable from the internet.
///
/// An IPv4-mapped IPv6 address is judged as its IPv4 address. Private, loopback, link-local,
/// unspecified, multicast, broadcast, "this network" (`0.0.0.0/8`), IETF protocol assignments
/// (`192.0.0.0/24`), benchmarking (`198.18.0.0/15`), reserved (`240.0.0.0/4`) and carrier-grade
/// NAT (`100.64.0.0/10`, RFC 6598) addresses are not public, and neither are IPv6 loopback,
/// unspecified, multicast, unique-local, link-local, site-local (`fec0::/10`), 6to4
/// (`2002::/16`) and the NAT64 well-known prefix (`64:ff9b::/96`) addresses. Documentation
/// ranges are not special-cased, because tests use them.
///
/// # Examples
///
/// ```
/// use shadowcat::portmap::status::is_public_address;
/// assert!(is_public_address("203.0.113.9".parse().unwrap()));
/// assert!(!is_public_address("192.168.1.1".parse().unwrap()));
/// assert!(!is_public_address("100.64.0.1".parse().unwrap()));
/// assert!(!is_public_address("::ffff:192.168.1.1".parse().unwrap()));
/// assert!(!is_public_address("198.18.0.1".parse().unwrap()));
/// ```
pub fn is_public_address(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(v4) => {
            let carrier_grade = v4.octets().first() == Some(&100)
                && v4.octets().get(1).is_some_and(|b| (*b & 0xc0) == 0x40);
            let octets = v4.octets();
            let this_network = octets.first() == Some(&0);
            let protocol_assignments = octets.get(..3) == Some(&[192, 0, 0]);
            let benchmarking =
                octets.first() == Some(&198) && octets.get(1).is_some_and(|b| (*b & 0xfe) == 18);
            let reserved = octets.first().is_some_and(|b| *b >= 240);
            !(this_network
                || protocol_assignments
                || benchmarking
                || reserved
                || v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_multicast()
                || v4.is_broadcast()
                || carrier_grade)
        }
        IpAddr::V6(v6) => {
            let first = v6.segments().first().copied().unwrap_or(0);
            let unique_local = (first & 0xfe00) == 0xfc00;
            let link_local = (first & 0xffc0) == 0xfe80;
            let site_local = (first & 0xffc0) == 0xfec0;
            let six_to_four = first == 0x2002;
            let nat64 = v6.segments().get(..6) == Some(&[0x64, 0xff9b, 0, 0, 0, 0]);
            !(site_local
                || six_to_four
                || nat64
                || v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || unique_local
                || link_local)
        }
    }
}
