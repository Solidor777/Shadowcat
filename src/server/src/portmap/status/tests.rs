//! Tests for the address classification behind the status.

use super::*;

#[test]
fn routable_addresses_are_public() {
    for ip in [
        "203.0.113.9",
        "8.8.8.8",
        "198.51.100.7",
        "2606:4700::1",
        "2001:db8::1",
    ] {
        assert!(is_public_address(ip.parse().unwrap()), "{ip}");
    }
}

#[test]
fn non_routable_ranges_are_not_public() {
    for ip in [
        "0.0.0.0",
        "0.1.2.3",
        "10.0.0.1",
        "100.64.0.1",
        "127.0.0.1",
        "169.254.1.1",
        "172.16.0.1",
        "192.0.0.9",
        "192.168.1.1",
        "198.18.0.1",
        "198.19.255.254",
        "224.0.0.1",
        "240.0.0.1",
        "255.255.255.255",
        "::",
        "::1",
        "fc00::1",
        "fe80::1",
        "fec0::1",
        "ff02::1",
        "2002:c000:204::1",
        "64:ff9b::c000:204",
    ] {
        assert!(!is_public_address(ip.parse().unwrap()), "{ip}");
    }
}

#[test]
fn an_ipv4_mapped_address_is_judged_as_its_ipv4_address() {
    assert!(!is_public_address("::ffff:192.168.1.1".parse().unwrap()));
    assert!(!is_public_address("::ffff:127.0.0.1".parse().unwrap()));
    assert!(is_public_address("::ffff:203.0.113.9".parse().unwrap()));
}
