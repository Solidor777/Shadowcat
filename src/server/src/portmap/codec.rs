//! Byte codec for NAT-PMP (RFC 6886) and PCP (RFC 6887) TCP port mappings.
//!
//! Encoders return the exact datagram to send. Decoders take the received datagram and return a
//! typed reply or a [`CodecError`]; they never panic and never read past the buffer. Source-address
//! filtering and retransmission live in the caller; this module only fixes the bytes and the
//! retry schedule ([`retry_delay`]).
//!
//! Layouts (all integers big-endian):
//!
//! - NAT-PMP request: version 0, opcode (0 external address, 2 TCP mapping). The mapping request is
//!   12 bytes: version, opcode, 2 reserved, internal port, suggested external port, lifetime.
//! - NAT-PMP reply: 12 bytes (address) or 16 bytes (mapping): version, opcode + 128, result code
//!   (2 bytes), seconds-since-start epoch, then the opcode body.
//! - PCP MAP request: 24-byte common header (version 2, opcode 1, 2 reserved, lifetime, client
//!   address as 16 bytes) plus a 36-byte MAP payload (nonce, protocol, 3 reserved, internal port,
//!   suggested external port, suggested external address as 16 bytes).
//! - PCP MAP reply: 24-byte header (version, opcode + 128, reserved, result code, lifetime, epoch,
//!   12 reserved) plus the 36-byte MAP payload with the assigned port and address.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Duration;

/// UDP port the gateway listens on for both NAT-PMP and PCP requests.
///
/// # Examples
///
/// ```
/// assert_eq!(shadowcat::portmap::codec::GATEWAY_PORT, 5351);
/// ```
pub const GATEWAY_PORT: u16 = 5351;

/// IANA protocol number for TCP, the only protocol the server maps.
///
/// # Examples
///
/// ```
/// assert_eq!(shadowcat::portmap::codec::PROTOCOL_TCP, 6);
/// ```
pub const PROTOCOL_TCP: u8 = 6;

/// Length in bytes of a PCP MAP request or reply (24-byte header plus 36-byte payload).
///
/// # Examples
///
/// ```
/// assert_eq!(shadowcat::portmap::codec::PCP_MAP_LEN, 60);
/// ```
pub const PCP_MAP_LEN: usize = 60;

/// Length in bytes of a PCP MAP nonce.
///
/// # Examples
///
/// ```
/// assert_eq!(shadowcat::portmap::codec::PCP_NONCE_LEN, 12);
/// ```
pub const PCP_NONCE_LEN: usize = 12;

/// Number of transmissions of one request before the gateway is declared silent. The RFC allows 9;
/// 5 resolves a missing gateway in seconds (250 ms, 500 ms, 1 s, 2 s, 4 s).
///
/// # Examples
///
/// ```
/// assert_eq!(shadowcat::portmap::codec::MAX_ATTEMPTS, 5);
/// ```
pub const MAX_ATTEMPTS: u32 = 5;

/// Wait after the first transmission; it doubles after every further one (RFC 6886 section 3.1).
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// assert_eq!(shadowcat::portmap::codec::INITIAL_RETRY, Duration::from_millis(250));
/// ```
pub const INITIAL_RETRY: Duration = Duration::from_millis(250);

/// NAT-PMP protocol version.
const NATPMP_VERSION: u8 = 0;
/// NAT-PMP opcode: report the external address.
const NATPMP_OP_ADDRESS: u8 = 0;
/// NAT-PMP opcode: map a TCP port.
const NATPMP_OP_MAP_TCP: u8 = 2;
/// Added to a request opcode to form the reply opcode.
const REPLY_OFFSET: u8 = 128;
/// NAT-PMP result code 1 and PCP result code 1: the peer does not speak the version sent.
const RESULT_UNSUPPORTED_VERSION: u8 = 1;
/// PCP protocol version.
const PCP_VERSION: u8 = 2;
/// PCP opcode for MAP.
const PCP_OP_MAP: u8 = 1;

/// Why a datagram could not be built into a typed reply.
///
/// # Examples
///
/// ```
/// use shadowcat::portmap::codec::{decode_natpmp_reply, CodecError};
/// assert_eq!(decode_natpmp_reply(&[0]), Err(CodecError::Truncated));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CodecError {
    /// The datagram ended before a field the format requires.
    #[error("datagram is shorter than the format requires")]
    Truncated,
    /// The datagram has the wrong total length for its opcode.
    #[error("datagram length {0} does not match the format")]
    BadLength(usize),
    /// The version byte is not the version the protocol attempt speaks.
    #[error("unexpected protocol version {0}")]
    WrongVersion(u8),
    /// The opcode is not the reply opcode this decoder accepts.
    #[error("unexpected opcode {0}")]
    WrongOpcode(u8),
    /// The reply's nonce differs from the nonce of the request it should answer.
    #[error("reply nonce does not match the request")]
    WrongNonce,
    /// The reply's protocol field is not TCP.
    #[error("reply protocol {0} is not TCP")]
    WrongProtocol(u8),
    /// The gateway answered result code 1: it does not support the version that was sent.
    #[error("gateway does not support this protocol version")]
    UnsupportedVersion,
    /// The gateway refused with a non-zero result code other than unsupported version.
    #[error("gateway refused with result code {0}")]
    ResultCode(u16),
}

/// Bounds-checked big-endian reader over a received datagram. Every read returns
/// [`CodecError::Truncated`] instead of indexing past the end.
struct Reader<'a> {
    /// The datagram being read.
    bytes: &'a [u8],
    /// Index of the next unread byte; never exceeds `bytes.len()`.
    pos: usize,
}

impl<'a> Reader<'a> {
    /// Starts reading at the first byte.
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    /// Takes the next `n` bytes, or fails if fewer remain.
    fn take(&mut self, n: usize) -> Result<&'a [u8], CodecError> {
        let end = self.pos.checked_add(n).ok_or(CodecError::Truncated)?;
        let slice = self.bytes.get(self.pos..end).ok_or(CodecError::Truncated)?;
        self.pos = end;
        Ok(slice)
    }

    /// Takes the next `N` bytes as an array.
    fn array<const N: usize>(&mut self) -> Result<[u8; N], CodecError> {
        self.take(N)?.try_into().map_err(|_| CodecError::Truncated)
    }

    /// Reads one byte.
    fn u8(&mut self) -> Result<u8, CodecError> {
        Ok(u8::from_be_bytes(self.array()?))
    }

    /// Reads a big-endian `u16`.
    fn u16(&mut self) -> Result<u16, CodecError> {
        Ok(u16::from_be_bytes(self.array()?))
    }

    /// Reads a big-endian `u32`.
    fn u32(&mut self) -> Result<u32, CodecError> {
        Ok(u32::from_be_bytes(self.array()?))
    }

    /// Succeeds only when every byte has been read; otherwise reports the total length as bad.
    fn finish(self) -> Result<(), CodecError> {
        if self.pos == self.bytes.len() {
            Ok(())
        } else {
            Err(CodecError::BadLength(self.bytes.len()))
        }
    }
}

/// A decoded NAT-PMP reply.
///
/// # Examples
///
/// ```
/// use std::net::Ipv4Addr;
/// use shadowcat::portmap::codec::{decode_natpmp_reply, NatPmpReply};
/// let reply = decode_natpmp_reply(&[0, 128, 0, 0, 0, 0, 0, 9, 192, 0, 2, 7]).unwrap();
/// assert_eq!(reply, NatPmpReply::ExternalAddress { epoch: 9, address: Ipv4Addr::new(192, 0, 2, 7) });
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NatPmpReply {
    /// Answer to an external-address request.
    ExternalAddress {
        /// Gateway seconds since start; see [`gateway_restarted`].
        epoch: u32,
        /// The gateway's external IPv4 address.
        address: Ipv4Addr,
    },
    /// Answer to a TCP mapping request (or a delete, which is a mapping with lifetime 0).
    Mapping {
        /// Gateway seconds since start; see [`gateway_restarted`].
        epoch: u32,
        /// The internal port the mapping is for.
        internal_port: u16,
        /// The external port the gateway assigned.
        external_port: u16,
        /// Granted lifetime in seconds.
        lifetime_secs: u32,
    },
}

/// A decoded PCP MAP reply.
///
/// # Examples
///
/// ```
/// use shadowcat::portmap::codec::{decode_pcp_map_reply, pcp_map_request};
/// // A reply is a request with the reply bit and a result set; this one is hand-built from a
/// // request to show the field layout.
/// let nonce = [7u8; 12];
/// let mut reply = pcp_map_request("192.168.1.5".parse().unwrap(), nonce, 8080, 8080, 3600);
/// reply[1] = 0x81;
/// reply[3] = 0;
/// let decoded = decode_pcp_map_reply(&reply, &nonce).unwrap();
/// assert_eq!(decoded.internal_port, 8080);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcpMapReply {
    /// Granted lifetime in seconds.
    pub lifetime_secs: u32,
    /// Gateway seconds since start; see [`gateway_restarted`].
    pub epoch: u32,
    /// The internal port the mapping is for.
    pub internal_port: u16,
    /// The external port the gateway assigned.
    pub external_port: u16,
    /// The external address the gateway assigned; IPv4-mapped addresses are returned as IPv4.
    pub external_address: IpAddr,
}

/// Encodes a NAT-PMP external-address request (2 bytes).
///
/// # Examples
///
/// ```
/// assert_eq!(shadowcat::portmap::codec::natpmp_external_address_request(), vec![0, 0]);
/// ```
pub fn natpmp_external_address_request() -> Vec<u8> {
    vec![NATPMP_VERSION, NATPMP_OP_ADDRESS]
}

/// Encodes a NAT-PMP TCP mapping request (12 bytes). A lifetime of 0 with an external port of 0
/// is a delete; use [`natpmp_delete_request`].
///
/// # Examples
///
/// ```
/// let req = shadowcat::portmap::codec::natpmp_map_request(8080, 8080, 3600);
/// assert_eq!(req, vec![0, 2, 0, 0, 0x1F, 0x90, 0x1F, 0x90, 0, 0, 0x0E, 0x10]);
/// ```
pub fn natpmp_map_request(
    internal_port: u16,
    suggested_external_port: u16,
    lifetime_secs: u32,
) -> Vec<u8> {
    let mut out = vec![NATPMP_VERSION, NATPMP_OP_MAP_TCP, 0, 0];
    out.extend_from_slice(&internal_port.to_be_bytes());
    out.extend_from_slice(&suggested_external_port.to_be_bytes());
    out.extend_from_slice(&lifetime_secs.to_be_bytes());
    out
}

/// Encodes a NAT-PMP delete: a TCP mapping request for `internal_port` with external port 0 and
/// lifetime 0 (RFC 6886 section 3.4).
///
/// # Examples
///
/// ```
/// let req = shadowcat::portmap::codec::natpmp_delete_request(8080);
/// assert_eq!(req, vec![0, 2, 0, 0, 0x1F, 0x90, 0, 0, 0, 0, 0, 0]);
/// ```
pub fn natpmp_delete_request(internal_port: u16) -> Vec<u8> {
    natpmp_map_request(internal_port, 0, 0)
}

/// Decodes a NAT-PMP reply. Accepts only the address reply (12 bytes) and the TCP mapping reply
/// (16 bytes). A non-zero result code is an error whatever follows it.
///
/// # Examples
///
/// ```
/// use shadowcat::portmap::codec::{decode_natpmp_reply, NatPmpReply};
/// let bytes = [0, 130, 0, 0, 0, 0, 0, 5, 0x1F, 0x90, 0x1F, 0x91, 0, 0, 0x0E, 0x10];
/// assert_eq!(
///     decode_natpmp_reply(&bytes).unwrap(),
///     NatPmpReply::Mapping { epoch: 5, internal_port: 8080, external_port: 8081, lifetime_secs: 3600 }
/// );
/// ```
pub fn decode_natpmp_reply(bytes: &[u8]) -> Result<NatPmpReply, CodecError> {
    let mut r = Reader::new(bytes);
    let version = r.u8()?;
    if version != NATPMP_VERSION {
        return Err(CodecError::WrongVersion(version));
    }
    let opcode = r.u8()?;
    let result = r.u16()?;
    if result != 0 {
        return Err(if result == u16::from(RESULT_UNSUPPORTED_VERSION) {
            CodecError::UnsupportedVersion
        } else {
            CodecError::ResultCode(result)
        });
    }
    let epoch = r.u32()?;
    let reply = match opcode.checked_sub(REPLY_OFFSET) {
        Some(NATPMP_OP_ADDRESS) => NatPmpReply::ExternalAddress {
            epoch,
            address: Ipv4Addr::from(r.array::<4>()?),
        },
        Some(NATPMP_OP_MAP_TCP) => NatPmpReply::Mapping {
            epoch,
            internal_port: r.u16()?,
            external_port: r.u16()?,
            lifetime_secs: r.u32()?,
        },
        _ => return Err(CodecError::WrongOpcode(opcode)),
    };
    r.finish()?;
    Ok(reply)
}

/// Encodes a PCP MAP request for a TCP mapping (60 bytes). `client` is the local address the
/// gateway sees the request from. The suggested external address is the wildcard (all zero), so the
/// gateway picks it. Reuse the same `nonce` for renewals and for the delete (lifetime 0).
///
/// # Examples
///
/// ```
/// let req = shadowcat::portmap::codec::pcp_map_request(
///     "192.168.1.5".parse().unwrap(), [1; 12], 8080, 8080, 3600,
/// );
/// assert_eq!(req.len(), 60);
/// assert_eq!(&req[..2], &[2, 1]);
/// ```
pub fn pcp_map_request(
    client: IpAddr,
    nonce: [u8; PCP_NONCE_LEN],
    internal_port: u16,
    suggested_external_port: u16,
    lifetime_secs: u32,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(PCP_MAP_LEN);
    out.extend_from_slice(&[PCP_VERSION, PCP_OP_MAP, 0, 0]);
    out.extend_from_slice(&lifetime_secs.to_be_bytes());
    out.extend_from_slice(&address_bytes(client));
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&[PROTOCOL_TCP, 0, 0, 0]);
    out.extend_from_slice(&internal_port.to_be_bytes());
    out.extend_from_slice(&suggested_external_port.to_be_bytes());
    out.extend_from_slice(&[0; 16]);
    out
}

/// Encodes a PCP delete: the MAP request that created the mapping, with lifetime 0 and the same
/// nonce (RFC 6887 section 15).
///
/// # Examples
///
/// ```
/// let req = shadowcat::portmap::codec::pcp_delete_request(
///     "192.168.1.5".parse().unwrap(), [1; 12], 8080,
/// );
/// assert_eq!(&req[4..8], &[0, 0, 0, 0]);
/// ```
pub fn pcp_delete_request(
    client: IpAddr,
    nonce: [u8; PCP_NONCE_LEN],
    internal_port: u16,
) -> Vec<u8> {
    pcp_map_request(client, nonce, internal_port, 0, 0)
}

/// Decodes a PCP MAP reply for the request that used `expected_nonce`.
///
/// A reply that is not version 2, not a MAP reply, carries a different nonce or protocol, or has
/// a non-zero result code is an error. The result code is checked before the payload, because a
/// gateway rejecting the version answers with the 24-byte header alone. `UnsupportedVersion`
/// tells the caller to fall back to NAT-PMP.
///
/// # Examples
///
/// ```
/// use shadowcat::portmap::codec::{decode_pcp_map_reply, CodecError};
/// let mut header = vec![2, 0x81, 0, 1];
/// header.resize(24, 0);
/// assert_eq!(decode_pcp_map_reply(&header, &[0; 12]), Err(CodecError::UnsupportedVersion));
/// ```
pub fn decode_pcp_map_reply(
    bytes: &[u8],
    expected_nonce: &[u8; PCP_NONCE_LEN],
) -> Result<PcpMapReply, CodecError> {
    let mut r = Reader::new(bytes);
    let version = r.u8()?;
    if version != PCP_VERSION {
        return Err(CodecError::WrongVersion(version));
    }
    let opcode = r.u8()?;
    r.u8()?;
    let result = r.u8()?;
    let lifetime_secs = r.u32()?;
    let epoch = r.u32()?;
    r.take(12)?;
    if result != 0 {
        return Err(if result == RESULT_UNSUPPORTED_VERSION {
            CodecError::UnsupportedVersion
        } else {
            CodecError::ResultCode(u16::from(result))
        });
    }
    if opcode != PCP_OP_MAP.wrapping_add(REPLY_OFFSET) {
        return Err(CodecError::WrongOpcode(opcode));
    }
    if r.array::<PCP_NONCE_LEN>()? != *expected_nonce {
        return Err(CodecError::WrongNonce);
    }
    let protocol = r.u8()?;
    if protocol != PROTOCOL_TCP {
        return Err(CodecError::WrongProtocol(protocol));
    }
    r.take(3)?;
    let internal_port = r.u16()?;
    let external_port = r.u16()?;
    let external_address = address_from_bytes(r.array::<16>()?);
    r.finish()?;
    Ok(PcpMapReply {
        lifetime_secs,
        epoch,
        internal_port,
        external_port,
        external_address,
    })
}

/// True when a gateway's reported epoch shows it restarted since the previous reply, so every
/// mapping it held is gone and must be requested again.
///
/// The epoch is the gateway's seconds since start. It restarted when it went backwards, or when it
/// advanced less than the client's own elapsed time minus 1/8 of that time and a 2-second margin
/// (the margin absorbs gateway clock skew; RFC 6886 section 3.6 describes the check).
///
/// # Examples
///
/// ```
/// use shadowcat::portmap::codec::gateway_restarted;
/// assert!(!gateway_restarted(1000, 1060, 60));
/// assert!(gateway_restarted(1000, 20, 60));
/// ```
pub fn gateway_restarted(previous_epoch: u32, current_epoch: u32, elapsed_secs: u64) -> bool {
    let Some(advanced) = current_epoch.checked_sub(previous_epoch) else {
        return true;
    };
    let floor = elapsed_secs
        .saturating_sub(elapsed_secs / 8)
        .saturating_sub(2);
    u64::from(advanced) < floor
}

/// Wait before retransmission number `attempt + 1`: 250 ms doubling, for `attempt` in
/// `0..MAX_ATTEMPTS`. `None` once the attempts are exhausted.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use shadowcat::portmap::codec::retry_delay;
/// assert_eq!(retry_delay(0), Some(Duration::from_millis(250)));
/// assert_eq!(retry_delay(2), Some(Duration::from_secs(1)));
/// assert_eq!(retry_delay(5), None);
/// ```
pub fn retry_delay(attempt: u32) -> Option<Duration> {
    if attempt >= MAX_ATTEMPTS {
        return None;
    }
    INITIAL_RETRY.checked_mul(1u32.checked_shl(attempt)?)
}

/// The 16-byte wire form of an address: IPv4 as an IPv4-mapped IPv6 address.
fn address_bytes(ip: IpAddr) -> [u8; 16] {
    match ip {
        IpAddr::V4(v4) => v4.to_ipv6_mapped().octets(),
        IpAddr::V6(v6) => v6.octets(),
    }
}

/// Inverse of [`address_bytes`]: an IPv4-mapped address becomes IPv4.
fn address_from_bytes(bytes: [u8; 16]) -> IpAddr {
    let v6 = Ipv6Addr::from(bytes);
    match v6.to_ipv4_mapped() {
        Some(v4) => IpAddr::V4(v4),
        None => IpAddr::V6(v6),
    }
}

#[cfg(test)]
mod tests;
