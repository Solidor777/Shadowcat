//! The UPnP IGD flow: discover gateways over SSDP, fetch and validate each device description,
//! then add, query and delete a TCP port mapping over SOAP.
//!
//! Trust is decided in one place, [`trusted_endpoint`]: both the `LOCATION` URL and the
//! resolved control URL must name the exact IP that answered discovery. The `accept` predicate
//! passed to [`discover`] fixes which addresses count as a LAN; production passes
//! `is_lan_address`.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use tokio::net::UdpSocket;
use url::Url;

use super::soap::{
    element_text, http_request, parse_description, soap_call, trusted_endpoint, Endpoint, IgdError,
    HTTP_DEADLINE,
};
use super::ssdp::{self, Responder};

/// Lease requested for a mapping, in seconds.
///
/// # Examples
///
/// ```
/// assert_eq!(shadowcat::portmap::igd::REQUESTED_LEASE_SECS, 3600);
/// ```
pub const REQUESTED_LEASE_SECS: u32 = 3600;

/// How many ports above the preferred one are tried after a conflict (code 718).
///
/// # Examples
///
/// ```
/// assert_eq!(shadowcat::portmap::igd::CONFLICT_WINDOW, 8);
/// ```
pub const CONFLICT_WINDOW: u16 = 8;

/// UPnP error: the gateway supports only permanent leases.
const ERR_PERMANENT_LEASE_ONLY: u32 = 725;

/// UPnP error: the external port is already mapped for another client.
const ERR_CONFLICT: u32 = 718;

/// Description of the mapping on the gateway.
const MAPPING_DESCRIPTION: &str = "Shadowcat";

/// A gateway that passed discovery, description and trust checks.
///
/// # Examples
///
/// ```no_run
/// use shadowcat::portmap::igd::discover;
/// use shadowcat::portmap::soap::is_lan_address;
/// use shadowcat::portmap::ssdp::{COLLECT_WINDOW, SSDP_GROUP};
/// # async fn demo() {
/// let gateways = discover(SSDP_GROUP, COLLECT_WINDOW, is_lan_address).await;
/// # let _ = gateways;
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct IgdGateway {
    /// The gateway's address, equal to its SSDP source address.
    pub responder: IpAddr,
    /// The validated control endpoint.
    control: Endpoint,
    /// The WAN service type, used verbatim in every request.
    service_type: String,
    /// Local address the gateway reaches this host on (`NewInternalClient`).
    local: IpAddr,
}

/// A mapping the gateway granted.
///
/// # Examples
///
/// ```
/// use shadowcat::portmap::igd::IgdMapping;
/// let mapping = IgdMapping { external_port: 8080, lease_secs: 0 };
/// assert_eq!(mapping.lease_secs, 0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IgdMapping {
    /// The external port actually granted.
    pub external_port: u16,
    /// Granted lease in seconds; 0 marks a permanent mapping that shutdown still deletes.
    pub lease_secs: u32,
}

/// Finds the local address the OS would use to reach `peer`, by connecting an unsent UDP socket.
async fn local_address_toward(peer: SocketAddr) -> Result<IpAddr, IgdError> {
    let bind = if peer.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    let socket = UdpSocket::bind(bind)
        .await
        .map_err(|e| IgdError::Io(e.kind()))?;
    socket
        .connect(peer)
        .await
        .map_err(|e| IgdError::Io(e.kind()))?;
    socket
        .local_addr()
        .map(|addr| addr.ip())
        .map_err(|e| IgdError::Io(e.kind()))
}

/// Fetches and validates one responder's device description.
async fn describe(
    responder: &Responder,
    accept: fn(IpAddr) -> bool,
) -> Result<IgdGateway, IgdError> {
    let location =
        Url::parse(&responder.location).map_err(|_| IgdError::Malformed("bad LOCATION"))?;
    let description_endpoint = trusted_endpoint(&location, responder.ip, accept)?;
    let reply = http_request(&description_endpoint, "GET", &[], b"", HTTP_DEADLINE).await?;
    if reply.status != 200 {
        return Err(IgdError::Status(reply.status));
    }
    let xml = String::from_utf8(reply.body)
        .map_err(|_| IgdError::Malformed("description is not text"))?;
    let service = parse_description(&xml)?;
    let base = service
        .url_base
        .as_deref()
        .and_then(|base| Url::parse(base).ok())
        .unwrap_or(location);
    let control_url = base
        .join(&service.control_url)
        .map_err(|_| IgdError::Malformed("bad controlURL"))?;
    let control = trusted_endpoint(&control_url, responder.ip, accept)?;
    let local = local_address_toward(control.addr).await?;
    Ok(IgdGateway {
        responder: responder.ip,
        control,
        service_type: service.service_type,
        local,
    })
}

/// Sends SSDP searches to `target`, then validates every responder's description.
///
/// # Errors
///
/// [`IgdError::NoResponse`] when nothing answered; otherwise the first per-responder error
/// (for example [`IgdError::Untrusted`]) when no responder produced a usable gateway.
///
/// # Examples
///
/// ```no_run
/// use shadowcat::portmap::igd::discover;
/// use shadowcat::portmap::soap::is_lan_address;
/// use shadowcat::portmap::ssdp::{COLLECT_WINDOW, SSDP_GROUP};
/// # async fn demo() {
/// let found = discover(SSDP_GROUP, COLLECT_WINDOW, is_lan_address).await;
/// # let _ = found;
/// # }
/// ```
pub async fn discover(
    target: SocketAddr,
    window: Duration,
    accept: fn(IpAddr) -> bool,
) -> Result<Vec<IgdGateway>, IgdError> {
    let responders = ssdp::discover_responders(target, window).await;
    let mut gateways = Vec::new();
    let mut first_error = None;
    for responder in &responders {
        match describe(responder, accept).await {
            Ok(gateway) => gateways.push(gateway),
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    if gateways.is_empty() {
        return Err(first_error.unwrap_or(IgdError::NoResponse));
    }
    Ok(gateways)
}

impl IgdGateway {
    /// Asks the gateway for its external address (`GetExternalIPAddress`).
    ///
    /// # Errors
    ///
    /// Any [`IgdError`]; [`IgdError::Malformed`] when the reply holds no parsable address.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn demo(gateway: shadowcat::portmap::igd::IgdGateway) {
    /// let address = gateway.external_address().await;
    /// # let _ = address;
    /// # }
    /// ```
    pub async fn external_address(&self) -> Result<IpAddr, IgdError> {
        let body = soap_call(
            &self.control,
            &self.service_type,
            "GetExternalIPAddress",
            &[],
        )
        .await?;
        element_text(&body, "NewExternalIPAddress")?
            .and_then(|text| text.parse().ok())
            .ok_or(IgdError::Malformed("no external address"))
    }

    /// Maps TCP `internal_port` to `preferred_external` for [`REQUESTED_LEASE_SECS`].
    ///
    /// Code 725 retries once with lease 0 (a permanent mapping, reported as `lease_secs` 0).
    /// Code 718 tries the next ports, up to [`CONFLICT_WINDOW`] above the preferred one.
    ///
    /// # Errors
    ///
    /// The last [`IgdError`]; [`IgdError::Fault`] carries the gateway's code.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn demo(gateway: shadowcat::portmap::igd::IgdGateway) {
    /// let mapping = gateway.add_mapping(8080, 8080).await;
    /// # let _ = mapping;
    /// # }
    /// ```
    pub async fn add_mapping(
        &self,
        internal_port: u16,
        preferred_external: u16,
    ) -> Result<IgdMapping, IgdError> {
        let mut lease = REQUESTED_LEASE_SECS;
        let mut offset: u16 = 0;
        loop {
            let Some(external) = preferred_external.checked_add(offset) else {
                return Err(IgdError::Fault(ERR_CONFLICT));
            };
            match self.try_add(internal_port, external, lease).await {
                Ok(()) => {
                    return Ok(IgdMapping {
                        external_port: external,
                        lease_secs: lease,
                    });
                }
                Err(IgdError::Fault(ERR_PERMANENT_LEASE_ONLY)) if lease != 0 => lease = 0,
                Err(IgdError::Fault(ERR_CONFLICT)) if offset < CONFLICT_WINDOW => {
                    offset = offset.saturating_add(1);
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Sends one `AddPortMapping`.
    async fn try_add(
        &self,
        internal_port: u16,
        external_port: u16,
        lease: u32,
    ) -> Result<(), IgdError> {
        let args = [
            ("NewRemoteHost", String::new()),
            ("NewExternalPort", external_port.to_string()),
            ("NewProtocol", "TCP".to_owned()),
            ("NewInternalPort", internal_port.to_string()),
            ("NewInternalClient", self.local.to_string()),
            ("NewEnabled", "1".to_owned()),
            ("NewPortMappingDescription", MAPPING_DESCRIPTION.to_owned()),
            ("NewLeaseDuration", lease.to_string()),
        ];
        soap_call(&self.control, &self.service_type, "AddPortMapping", &args)
            .await
            .map(|_| ())
    }

    /// Deletes the TCP mapping on `external_port`.
    ///
    /// # Errors
    ///
    /// Any [`IgdError`].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn demo(gateway: shadowcat::portmap::igd::IgdGateway) {
    /// let result = gateway.delete_mapping(8080).await;
    /// # let _ = result;
    /// # }
    /// ```
    pub async fn delete_mapping(&self, external_port: u16) -> Result<(), IgdError> {
        let args = [
            ("NewRemoteHost", String::new()),
            ("NewExternalPort", external_port.to_string()),
            ("NewProtocol", "TCP".to_owned()),
        ];
        soap_call(
            &self.control,
            &self.service_type,
            "DeletePortMapping",
            &args,
        )
        .await
        .map(|_| ())
    }
}

#[cfg(test)]
mod tests;
