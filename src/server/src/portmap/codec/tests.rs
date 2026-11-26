//! Codec tests: exact encodings, round trips through hand-built replies, a malformed-input table
//! per protocol, and exhaustive truncation/mutation sweeps proving the decoders return an error
//! (or a value) and never panic.

use super::*;

/// A fixed nonce for requests and matching replies.
const NONCE: [u8; PCP_NONCE_LEN] = [9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 11, 12];

/// The client address used in PCP requests.
fn client() -> IpAddr {
    "192.0.2.10".parse().unwrap()
}

/// A valid NAT-PMP mapping reply: epoch 5, internal 8080, external 8081, lifetime 3600.
fn natpmp_mapping_reply() -> Vec<u8> {
    vec![
        0, 130, 0, 0, 0, 0, 0, 5, 0x1F, 0x90, 0x1F, 0x91, 0, 0, 0x0E, 0x10,
    ]
}

/// A valid NAT-PMP external-address reply: epoch 9, address 192.0.2.7.
fn natpmp_address_reply() -> Vec<u8> {
    vec![0, 128, 0, 0, 0, 0, 0, 9, 192, 0, 2, 7]
}

/// A valid PCP MAP reply built by turning the matching request into a reply: reply bit set,
/// result 0, lifetime 3600, epoch 77, external port 8081, external address 198.51.100.4.
fn pcp_reply() -> Vec<u8> {
    let mut b = pcp_map_request(client(), NONCE, 8080, 8081, 3600);
    b[1] = 0x81;
    b[3] = 0;
    b[8..12].copy_from_slice(&77u32.to_be_bytes());
    let ext: IpAddr = "198.51.100.4".parse().unwrap();
    b[44..60].copy_from_slice(&address_bytes(ext));
    b
}

#[test]
fn natpmp_requests_have_the_exact_wire_bytes() {
    assert_eq!(natpmp_external_address_request(), vec![0, 0]);
    assert_eq!(
        natpmp_map_request(8080, 8081, 3600),
        vec![0, 2, 0, 0, 0x1F, 0x90, 0x1F, 0x91, 0, 0, 0x0E, 0x10]
    );
    assert_eq!(
        natpmp_delete_request(8080),
        vec![0, 2, 0, 0, 0x1F, 0x90, 0, 0, 0, 0, 0, 0]
    );
}

#[test]
fn natpmp_replies_decode() {
    assert_eq!(
        decode_natpmp_reply(&natpmp_address_reply()).unwrap(),
        NatPmpReply::ExternalAddress {
            epoch: 9,
            address: Ipv4Addr::new(192, 0, 2, 7)
        }
    );
    assert_eq!(
        decode_natpmp_reply(&natpmp_mapping_reply()).unwrap(),
        NatPmpReply::Mapping {
            epoch: 5,
            internal_port: 8080,
            external_port: 8081,
            lifetime_secs: 3600
        }
    );
}

#[test]
fn natpmp_malformed_replies_are_errors() {
    let mut wrong_version = natpmp_mapping_reply();
    wrong_version[0] = 1;
    let mut udp_opcode = natpmp_mapping_reply();
    udp_opcode[1] = 129;
    let mut request_opcode = natpmp_mapping_reply();
    request_opcode[1] = 2;
    let mut address_with_mapping_length = natpmp_address_reply();
    address_with_mapping_length.extend_from_slice(&[0, 0, 0, 0]);
    let mut mapping_with_address_length = natpmp_mapping_reply();
    mapping_with_address_length.truncate(12);
    let mut long = natpmp_mapping_reply();
    long.push(0);
    let mut refused = natpmp_mapping_reply();
    refused[3] = 2;
    let mut unsupported = natpmp_mapping_reply();
    unsupported[3] = 1;
    let header_only_refusal = vec![0, 130, 0, 3];

    let cases: Vec<(&str, Vec<u8>, CodecError)> = vec![
        ("empty", vec![], CodecError::Truncated),
        ("one byte", vec![0], CodecError::Truncated),
        (
            "short",
            natpmp_mapping_reply()[..7].to_vec(),
            CodecError::Truncated,
        ),
        ("wrong version", wrong_version, CodecError::WrongVersion(1)),
        ("udp opcode", udp_opcode, CodecError::WrongOpcode(129)),
        ("request opcode", request_opcode, CodecError::WrongOpcode(2)),
        (
            "address reply too long",
            address_with_mapping_length,
            CodecError::BadLength(16),
        ),
        (
            "mapping reply too short",
            mapping_with_address_length,
            CodecError::Truncated,
        ),
        ("trailing byte", long, CodecError::BadLength(17)),
        ("refused", refused, CodecError::ResultCode(2)),
        (
            "unsupported version",
            unsupported,
            CodecError::UnsupportedVersion,
        ),
        (
            "refusal without body",
            header_only_refusal,
            CodecError::ResultCode(3),
        ),
    ];
    for (name, bytes, want) in cases {
        assert_eq!(decode_natpmp_reply(&bytes), Err(want), "case: {name}");
    }
}

#[test]
fn pcp_request_has_the_exact_wire_layout() {
    let req = pcp_map_request(client(), NONCE, 8080, 8081, 3600);
    assert_eq!(req.len(), PCP_MAP_LEN);
    assert_eq!(&req[..4], &[2, 1, 0, 0]);
    assert_eq!(&req[4..8], &3600u32.to_be_bytes());
    assert_eq!(&req[8..24], &client_mapped());
    assert_eq!(&req[24..36], &NONCE);
    assert_eq!(&req[36..40], &[6, 0, 0, 0]);
    assert_eq!(&req[40..42], &8080u16.to_be_bytes());
    assert_eq!(&req[42..44], &8081u16.to_be_bytes());
    assert_eq!(&req[44..60], &[0u8; 16]);
}

/// `client()` as an IPv4-mapped IPv6 address.
fn client_mapped() -> [u8; 16] {
    let mut b = [0u8; 16];
    b[10] = 0xff;
    b[11] = 0xff;
    b[12..].copy_from_slice(&[192, 0, 2, 10]);
    b
}

#[test]
fn pcp_ipv6_client_is_written_unmapped() {
    let v6: IpAddr = "2001:db8::5".parse().unwrap();
    let req = pcp_map_request(v6, NONCE, 1, 1, 1);
    assert_eq!(
        &req[8..24],
        &"2001:db8::5".parse::<Ipv6Addr>().unwrap().octets()
    );
}

#[test]
fn pcp_delete_is_the_map_request_with_lifetime_zero_and_the_same_nonce() {
    let del = pcp_delete_request(client(), NONCE, 8080);
    assert_eq!(&del[4..8], &[0, 0, 0, 0]);
    assert_eq!(&del[24..36], &NONCE);
    assert_eq!(&del[42..44], &[0, 0]);
}

#[test]
fn pcp_reply_decodes_and_unmaps_an_ipv4_address() {
    let got = decode_pcp_map_reply(&pcp_reply(), &NONCE).unwrap();
    assert_eq!(
        got,
        PcpMapReply {
            lifetime_secs: 3600,
            epoch: 77,
            internal_port: 8080,
            external_port: 8081,
            external_address: "198.51.100.4".parse().unwrap(),
        }
    );
}

#[test]
fn pcp_reply_keeps_a_native_ipv6_address() {
    let mut b = pcp_reply();
    let v6: Ipv6Addr = "2001:db8::9".parse().unwrap();
    b[44..60].copy_from_slice(&v6.octets());
    let got = decode_pcp_map_reply(&b, &NONCE).unwrap();
    assert_eq!(got.external_address, IpAddr::V6(v6));
}

#[test]
fn pcp_malformed_replies_are_errors() {
    let mutate = |f: &dyn Fn(&mut Vec<u8>)| {
        let mut b = pcp_reply();
        f(&mut b);
        b
    };
    let cases: Vec<(&str, Vec<u8>, CodecError)> = vec![
        ("empty", vec![], CodecError::Truncated),
        (
            "header only, result ok",
            pcp_reply()[..24].to_vec(),
            CodecError::Truncated,
        ),
        (
            "truncated payload",
            pcp_reply()[..59].to_vec(),
            CodecError::Truncated,
        ),
        (
            "trailing byte",
            mutate(&|b| b.push(0)),
            CodecError::BadLength(61),
        ),
        (
            "wrong version",
            mutate(&|b| b[0] = 1),
            CodecError::WrongVersion(1),
        ),
        (
            "version zero",
            mutate(&|b| b[0] = 0),
            CodecError::WrongVersion(0),
        ),
        (
            "request bit not set",
            mutate(&|b| b[1] = 1),
            CodecError::WrongOpcode(1),
        ),
        (
            "wrong opcode",
            mutate(&|b| b[1] = 0x82),
            CodecError::WrongOpcode(0x82),
        ),
        (
            "wrong nonce",
            mutate(&|b| b[24] ^= 0xFF),
            CodecError::WrongNonce,
        ),
        (
            "udp protocol",
            mutate(&|b| b[36] = 17),
            CodecError::WrongProtocol(17),
        ),
        ("refused", mutate(&|b| b[3] = 8), CodecError::ResultCode(8)),
        (
            "unsupported version",
            mutate(&|b| b[3] = 1),
            CodecError::UnsupportedVersion,
        ),
    ];
    for (name, bytes, want) in cases {
        assert_eq!(
            decode_pcp_map_reply(&bytes, &NONCE),
            Err(want),
            "case: {name}"
        );
    }
}

#[test]
fn pcp_unsupported_version_is_recognised_from_the_header_alone() {
    let mut header = vec![2, 0x81, 0, 1];
    header.resize(24, 0);
    assert_eq!(
        decode_pcp_map_reply(&header, &NONCE),
        Err(CodecError::UnsupportedVersion)
    );
}

#[test]
fn a_pcp_reply_for_another_request_is_rejected_not_accepted() {
    let other = [0u8; PCP_NONCE_LEN];
    assert_eq!(
        decode_pcp_map_reply(&pcp_reply(), &other),
        Err(CodecError::WrongNonce)
    );
}

#[test]
fn decoders_never_panic_on_any_prefix_or_single_byte_mutation() {
    let natpmp = [natpmp_address_reply(), natpmp_mapping_reply()];
    for valid in &natpmp {
        for len in 0..=valid.len() + 2 {
            let mut b = valid.clone();
            b.resize(len, 0xAA);
            let _ = decode_natpmp_reply(&b);
        }
        for pos in 0..valid.len() {
            for value in 0..=255u8 {
                let mut b = valid.clone();
                b[pos] = value;
                let _ = decode_natpmp_reply(&b);
            }
        }
    }
    let valid = pcp_reply();
    for len in 0..=valid.len() + 2 {
        let mut b = valid.clone();
        b.resize(len, 0xAA);
        let _ = decode_pcp_map_reply(&b, &NONCE);
    }
    for pos in 0..valid.len() {
        for value in 0..=255u8 {
            let mut b = valid.clone();
            b[pos] = value;
            let _ = decode_pcp_map_reply(&b, &NONCE);
        }
    }
}

#[test]
fn restart_detection_follows_elapsed_time() {
    assert!(!gateway_restarted(1000, 1060, 60), "in step");
    assert!(
        !gateway_restarted(1000, 1055, 60),
        "within the 1/8 plus 2 s tolerance"
    );
    assert!(
        gateway_restarted(1000, 1040, 60),
        "advanced far less than elapsed"
    );
    assert!(gateway_restarted(1000, 999, 0), "epoch went backwards");
    assert!(!gateway_restarted(0, 0, 1), "fresh reply inside the margin");
    assert!(
        !gateway_restarted(1000, 2000, 60),
        "advancing faster is not a restart"
    );
    assert!(
        gateway_restarted(0, u32::MAX, u64::MAX),
        "extreme values saturate, not overflow, and a huge elapsed time with a small advance is a restart"
    );
}
