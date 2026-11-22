//! UDP exchanges with the default gateway for NAT-PMP and PCP.
//!
//! Each exchange uses a socket connected to the gateway, so the operating system drops every
//! datagram from another source address: a hostile LAN peer cannot answer for the gateway.
//! Requests are retransmitted on the schedule of [`RetryPolicy`] (RFC 6886 section 3.1).

use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::time::Instant;

use super::codec::{
    decode_natpmp_reply, decode_pcp_map_reply, natpmp_delete_request,
    natpmp_external_address_request, natpmp_map_request, pcp_delete_request, pcp_map_request,
    CodecError, NatPmpReply, PCP_NONCE_LEN,
};

/// Largest datagram read from the gateway. PCP limits a message to 1100 bytes (RFC 6887
/// section 7); a longer one is truncated and then fails to decode.
const MAX_DATAGRAM: usize = 1100;

/// Why an exchange with the gateway failed.
///
/// # Examples
///
/// ```
/// use shadowcat::portmap::client::ClientError;
/// assert_eq!(ClientError::Timeout.to_string(), "the gateway did not answer");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClientError {
    /// A socket operation failed.
    #[error("socket error: {0:?}")]
    Io(ErrorKind),
    /// Nothing answered: the retransmissions ran out, or the host answered that no service listens.
    #[error("the gateway did not answer")]
    Timeout,
    /// The gateway answered with a refusal or an unsupported version.
    #[error("the gateway refused: {0}")]
    Refused(CodecError),
    /// The gateway granted a mapping that does not match the request.
    #[error("the gateway's grant does not match the request")]
    Mismatch,
}

/// Retransmission schedule of one request: the wait after the first transmission and how many
/// transmissions are sent. The wait doubles after each transmission.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use shadowcat::portmap::client::RetryPolicy;
/// let policy = RetryPolicy::PRODUCTION;
/// assert_eq!(policy.delay(1), Some(Duration::from_millis(500)));
/// assert_eq!(policy.delay(5), None);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Wait after the first transmission.
    pub initial: Duration,
    /// Number of transmissions before the gateway is declared silent.
    pub attempts: u32,
}

impl RetryPolicy {
    /// The RFC schedule capped at [`MAX_ATTEMPTS`](super::codec::MAX_ATTEMPTS) transmissions.
    ///
    /// # Examples
    ///
    /// ```
    /// assert_eq!(shadowcat::portmap::client::RetryPolicy::PRODUCTION.attempts, 5);
    /// ```
    pub const PRODUCTION: Self = Self {
        initial: super::codec::INITIAL_RETRY,
        attempts: super::codec::MAX_ATTEMPTS,
    };

    /// Wait after transmission number `attempt + 1`; `None` once `attempt` reaches the cap.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    /// use shadowcat::portmap::client::RetryPolicy;
    /// assert_eq!(RetryPolicy::PRODUCTION.delay(0), Some(Duration::from_millis(250)));
    /// ```
    pub fn delay(&self, attempt: u32) -> Option<Duration> {
        if attempt >= self.attempts {
            return None;
        }
        self.initial.checked_mul(1u32.checked_shl(attempt)?)
    }
}

/// A NAT-PMP mapping grant.
///
/// # Examples
///
/// ```
/// use shadowcat::portmap::client::PmpGrant;
/// let grant = PmpGrant { external_port: 8080, lifetime_secs: 3600, epoch: 1 };
/// assert_eq!(grant.external_port, 8080);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmpGrant {
    /// The external port the gateway assigned.
    pub external_port: u16,
    /// Granted lifetime in seconds.
    pub lifetime_secs: u32,
    /// The gateway's seconds since start.
    pub epoch: u32,
}

/// A PCP mapping grant.
///
/// # Examples
///
/// ```
/// use shadowcat::portmap::client::PcpGrant;
/// let grant = PcpGrant { external_port: 8080, lifetime_secs: 3600, epoch: 1, external_address: "203.0.113.5".parse().unwrap() };
/// assert_eq!(grant.lifetime_secs, 3600);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcpGrant {
    /// The external port the gateway assigned.
    pub external_port: u16,
    /// Granted lifetime in seconds.
    pub lifetime_secs: u32,
    /// The gateway's seconds since start.
    pub epoch: u32,
    /// The external address the gateway assigned.
    pub external_address: IpAddr,
}

/// Opens a UDP socket connected to `gateway`.
async fn connect(gateway: SocketAddr) -> Result<UdpSocket, ClientError> {
    let bind = if gateway.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    let socket = UdpSocket::bind(bind)
        .await
        .map_err(|e| ClientError::Io(e.kind()))?;
    socket
        .connect(gateway)
        .await
        .map_err(|e| ClientError::Io(e.kind()))?;
    Ok(socket)
}

/// True for the decode failures that mean the gateway answered "no": retransmitting cannot help.
fn is_refusal(error: &CodecError) -> bool {
    matches!(
        error,
        CodecError::ResultCode(_) | CodecError::UnsupportedVersion
    )
}

/// Sends `request` and waits for a reply `decode` accepts, retransmitting on `policy`.
///
/// A datagram that fails to decode for a reason other than a refusal (truncated, wrong nonce,
/// wrong opcode) is ignored and the wait continues, so a stray packet cannot end the exchange.
/// An ICMP "port unreachable" (surfaced by the OS as a refused or reset connection) ends it at once
/// as [`ClientError::Timeout`]: nothing listens there.
async fn exchange<T>(
    socket: &UdpSocket,
    request: &[u8],
    policy: RetryPolicy,
    decode: impl Fn(&[u8]) -> Result<T, CodecError>,
) -> Result<T, ClientError> {
    let mut buf = [0u8; MAX_DATAGRAM];
    let mut attempt = 0u32;
    while let Some(wait) = policy.delay(attempt) {
        socket
            .send(request)
            .await
            .map_err(|e| unreachable_or_io(e.kind()))?;
        let deadline = Instant::now()
            .checked_add(wait)
            .ok_or(ClientError::Timeout)?;
        loop {
            match tokio::time::timeout_at(deadline, socket.recv(&mut buf)).await {
                Err(_) => break,
                Ok(Err(e)) => return Err(unreachable_or_io(e.kind())),
                Ok(Ok(n)) => match decode(buf.get(..n).unwrap_or(&[])) {
                    Ok(value) => return Ok(value),
                    Err(error) if is_refusal(&error) => return Err(ClientError::Refused(error)),
                    Err(_) => {}
                },
            }
        }
        attempt = attempt.saturating_add(1);
    }
    Err(ClientError::Timeout)
}

/// Maps the "nothing listens" socket errors to [`ClientError::Timeout`] and the rest to `Io`.
fn unreachable_or_io(kind: ErrorKind) -> ClientError {
    match kind {
        ErrorKind::ConnectionRefused | ErrorKind::ConnectionReset => ClientError::Timeout,
        other => ClientError::Io(other),
    }
}

/// Asks a NAT-PMP gateway for its external IPv4 address and epoch.
///
/// # Errors
///
/// [`ClientError::Timeout`] when the gateway is silent, [`ClientError::Refused`] on a refusal.
///
/// # Examples
///
/// ```no_run
/// # async fn demo() {
/// use shadowcat::portmap::client::{natpmp_external_address, RetryPolicy};
/// let gateway = "192.168.1.1:5351".parse().unwrap();
/// let reply = natpmp_external_address(gateway, RetryPolicy::PRODUCTION).await;
/// # let _ = reply;
/// # }
/// ```
pub async fn natpmp_external_address(
    gateway: SocketAddr,
    policy: RetryPolicy,
) -> Result<(Ipv4Addr, u32), ClientError> {
    let socket = connect(gateway).await?;
    exchange(
        &socket,
        &natpmp_external_address_request(),
        policy,
        |bytes| match decode_natpmp_reply(bytes)? {
            NatPmpReply::ExternalAddress { epoch, address } => Ok((address, epoch)),
            NatPmpReply::Mapping { .. } => Err(CodecError::WrongOpcode(130)),
        },
    )
    .await
}

/// Requests (or renews) a NAT-PMP TCP mapping for `internal_port`.
///
/// # Errors
///
/// [`ClientError::Timeout`], [`ClientError::Refused`], or [`ClientError::Mismatch`] when the grant
/// names another internal port.
///
/// # Examples
///
/// ```no_run
/// # async fn demo() {
/// use shadowcat::portmap::client::{natpmp_map, RetryPolicy};
/// let gateway = "192.168.1.1:5351".parse().unwrap();
/// let grant = natpmp_map(gateway, 8080, 8080, 3600, RetryPolicy::PRODUCTION).await;
/// # let _ = grant;
/// # }
/// ```
pub async fn natpmp_map(
    gateway: SocketAddr,
    internal_port: u16,
    suggested_external: u16,
    lifetime_secs: u32,
    policy: RetryPolicy,
) -> Result<PmpGrant, ClientError> {
    let socket = connect(gateway).await?;
    let request = natpmp_map_request(internal_port, suggested_external, lifetime_secs);
    let grant = exchange(&socket, &request, policy, decode_natpmp_mapping).await?;
    if grant.0 != internal_port {
        return Err(ClientError::Mismatch);
    }
    Ok(grant.1)
}

/// Decodes a NAT-PMP mapping reply into its internal port and grant.
fn decode_natpmp_mapping(bytes: &[u8]) -> Result<(u16, PmpGrant), CodecError> {
    match decode_natpmp_reply(bytes)? {
        NatPmpReply::Mapping {
            epoch,
            internal_port,
            external_port,
            lifetime_secs,
        } => Ok((
            internal_port,
            PmpGrant {
                external_port,
                lifetime_secs,
                epoch,
            },
        )),
        NatPmpReply::ExternalAddress { .. } => Err(CodecError::WrongOpcode(128)),
    }
}

/// Deletes the NAT-PMP TCP mapping for `internal_port`.
///
/// # Errors
///
/// [`ClientError::Timeout`] or [`ClientError::Refused`].
///
/// # Examples
///
/// ```no_run
/// # async fn demo() {
/// use shadowcat::portmap::client::{natpmp_delete, RetryPolicy};
/// let gateway = "192.168.1.1:5351".parse().unwrap();
/// let result = natpmp_delete(gateway, 8080, RetryPolicy::PRODUCTION).await;
/// # let _ = result;
/// # }
/// ```
pub async fn natpmp_delete(
    gateway: SocketAddr,
    internal_port: u16,
    policy: RetryPolicy,
) -> Result<(), ClientError> {
    let socket = connect(gateway).await?;
    exchange(
        &socket,
        &natpmp_delete_request(internal_port),
        policy,
        decode_natpmp_mapping,
    )
    .await
    .map(|_| ())
}

/// Requests (or renews) a PCP TCP mapping for `internal_port`, using `nonce` to identify it.
///
/// # Errors
///
/// [`ClientError::Timeout`], [`ClientError::Refused`], or [`ClientError::Mismatch`] when the grant
/// names another internal port.
///
/// # Examples
///
/// ```no_run
/// # async fn demo() {
/// use shadowcat::portmap::client::{pcp_map, RetryPolicy};
/// let gateway = "192.168.1.1:5351".parse().unwrap();
/// let grant = pcp_map(gateway, [3; 12], 8080, 8080, 3600, RetryPolicy::PRODUCTION).await;
/// # let _ = grant;
/// # }
/// ```
pub async fn pcp_map(
    gateway: SocketAddr,
    nonce: [u8; PCP_NONCE_LEN],
    internal_port: u16,
    suggested_external: u16,
    lifetime_secs: u32,
    policy: RetryPolicy,
) -> Result<PcpGrant, ClientError> {
    let socket = connect(gateway).await?;
    let client = socket
        .local_addr()
        .map_err(|e| ClientError::Io(e.kind()))?
        .ip();
    let request = pcp_map_request(
        client,
        nonce,
        internal_port,
        suggested_external,
        lifetime_secs,
    );
    let reply = exchange(&socket, &request, policy, |bytes| {
        decode_pcp_map_reply(bytes, &nonce)
    })
    .await?;
    if reply.internal_port != internal_port {
        return Err(ClientError::Mismatch);
    }
    Ok(PcpGrant {
        external_port: reply.external_port,
        lifetime_secs: reply.lifetime_secs,
        epoch: reply.epoch,
        external_address: reply.external_address,
    })
}

/// Deletes the PCP TCP mapping identified by `nonce`.
///
/// # Errors
///
/// [`ClientError::Timeout`] or [`ClientError::Refused`].
///
/// # Examples
///
/// ```no_run
/// # async fn demo() {
/// use shadowcat::portmap::client::{pcp_delete, RetryPolicy};
/// let gateway = "192.168.1.1:5351".parse().unwrap();
/// let result = pcp_delete(gateway, [3; 12], 8080, RetryPolicy::PRODUCTION).await;
/// # let _ = result;
/// # }
/// ```
pub async fn pcp_delete(
    gateway: SocketAddr,
    nonce: [u8; PCP_NONCE_LEN],
    internal_port: u16,
    policy: RetryPolicy,
) -> Result<(), ClientError> {
    let socket = connect(gateway).await?;
    let client = socket
        .local_addr()
        .map_err(|e| ClientError::Io(e.kind()))?
        .ip();
    let request = pcp_delete_request(client, nonce, internal_port);
    exchange(&socket, &request, policy, |bytes| {
        decode_pcp_map_reply(bytes, &nonce)
    })
    .await
    .map(|_| ())
}

#[cfg(test)]
mod tests;
