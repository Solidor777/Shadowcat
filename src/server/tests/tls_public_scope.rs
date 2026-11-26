//! The self-signed certificate shown to public peers: which addresses it lists, how it persists
//! and when it is replaced. The peer-address selection over real handshakes lives in the
//! `tls::listener` unit tests.

use std::net::IpAddr;

use shadowcat::tls::resolver::PeerScope;
use shadowcat::tls::selfsigned::{
    force_regenerate_public, load_or_generate, load_or_generate_host, load_or_generate_public,
    regen_reason_for_host, regen_reason_public, RcgenGenerator, RegenReason, SanSet, SanSources,
    HOST_CERT_FILE, HOST_KEY_FILE, PUBLIC_CERT_FILE, PUBLIC_KEY_FILE, VALIDITY_DAYS,
};
use time::{Duration, OffsetDateTime};

fn now() -> OffsetDateTime {
    OffsetDateTime::now_utc()
}

fn ip(s: &str) -> IpAddr {
    s.parse().unwrap()
}

/// A trash that removes outright: these tests never inspect trashed files.
fn test_trash() -> shadowcat::trash::Trash {
    shadowcat::trash::Trash::new(
        std::env::temp_dir().join("shadowcat-tls-public-scope-trash"),
        shadowcat::trash::TrashRetention::Disabled,
    )
}

/// A host with a private and a global interface address and a hostname that embeds a name.
fn host(advertise: bool) -> SanSources {
    SanSources {
        machine_hostname: Some("Alice-Laptop".into()),
        advertise_hostname: advertise,
        interface_ips: vec![ip("192.0.2.20"), ip("198.51.100.50"), ip("fd00::5")],
    }
}

#[test]
fn the_scope_follows_the_peer_address_and_nothing_else() {
    for lan in [
        "10.0.0.1",
        "172.16.0.1",
        "172.31.255.254",
        "192.168.0.1",
        "169.254.1.1",
        "fc00::1",
        "fd12:3456::1",
        "fe80::1",
        "::ffff:192.168.1.5",
    ] {
        assert_eq!(PeerScope::of(ip(lan), false), PeerScope::Lan, "{lan}");
    }
    for public in [
        "203.0.113.9",
        "8.8.8.8",
        "172.32.0.1",
        "100.64.0.1",
        "192.0.0.1",
        "2001:db8::1",
        "2606:4700::1",
        "::ffff:203.0.113.9",
    ] {
        assert_eq!(
            PeerScope::of(ip(public), false),
            PeerScope::Public,
            "{public}"
        );
    }
}

#[test]
fn a_listed_trusted_proxy_is_public_whatever_it_looks_like() {
    for shared in [
        "127.0.0.1",
        "::1",
        "::ffff:127.0.0.1",
        "10.0.0.9",
        "192.168.1.1",
        "fd00::1",
    ] {
        assert_eq!(
            PeerScope::of(ip(shared), true),
            PeerScope::Public,
            "{shared}"
        );
    }
}

#[test]
fn a_loopback_peer_is_the_host_machine_unless_it_is_a_listed_proxy() {
    for loopback in ["127.0.0.1", "127.9.9.9", "::1", "::ffff:127.0.0.1"] {
        assert_eq!(
            PeerScope::of(ip(loopback), false),
            PeerScope::Host,
            "{loopback}"
        );
    }
}

#[test]
fn the_host_san_set_lists_localhost_names_only() {
    let host = SanSet::assemble_host();
    assert_eq!(host.dns, vec!["localhost".to_string()]);
    assert_eq!(host.ips, vec![ip("127.0.0.1"), ip("::1")]);
}

#[test]
fn the_host_certificate_persists_names_nothing_configured_and_is_reused() {
    let dir = tempfile::tempdir().unwrap();
    let t = now();
    let first = load_or_generate_host(Some(dir.path()), &RcgenGenerator, t, &test_trash()).unwrap();
    assert!(first.persisted);
    assert!(dir.path().join(HOST_CERT_FILE).is_file());
    assert!(dir.path().join(HOST_KEY_FILE).is_file());
    let mut entries = first.cert.san_entries();
    entries.sort();
    assert_eq!(entries, ["127.0.0.1", "::1", "localhost"]);
    let again = load_or_generate_host(Some(dir.path()), &RcgenGenerator, t, &test_trash()).unwrap();
    assert_eq!(first.cert.fingerprint(), again.cert.fingerprint());
    assert_eq!(regen_reason_for_host(&again.cert, t.unix_timestamp()), None);
    let near_expiry = (t + Duration::days(VALIDITY_DAYS - 1)).unix_timestamp();
    assert_eq!(
        regen_reason_for_host(&again.cert, near_expiry),
        Some(RegenReason::NearExpiry)
    );
    let replaced = load_or_generate_host(
        Some(dir.path()),
        &RcgenGenerator,
        t + Duration::days(VALIDITY_DAYS - 1),
        &test_trash(),
    )
    .unwrap();
    assert_ne!(first.cert.fingerprint(), replaced.cert.fingerprint());
}

#[test]
fn the_public_san_set_lists_no_interface_address_and_keeps_the_rest() {
    let sources = host(false);
    let hostnames = vec!["game.example".to_string(), "192.0.2.77".to_string()];
    let public = SanSet::assemble_public(&hostnames, &sources, Some(ip("203.0.113.9")));
    for interface in &sources.interface_ips {
        assert!(!public.ips.contains(interface), "{interface}");
    }
    assert!(public.dns.contains(&"localhost".to_string()));
    assert!(public.dns.contains(&"game.example".to_string()));
    assert!(public.ips.contains(&ip("127.0.0.1")) && public.ips.contains(&ip("::1")));
    assert!(
        public.ips.contains(&ip("192.0.2.77")),
        "an address the operator named in tls_hostnames is theirs"
    );
    assert!(public.ips.contains(&ip("203.0.113.9")));
    let lan = SanSet::assemble(&hostnames, &sources);
    for interface in &sources.interface_ips {
        assert!(lan.ips.contains(interface), "the LAN set keeps {interface}");
    }
    assert!(
        !lan.ips.contains(&ip("203.0.113.9")),
        "the external address belongs to the public certificate only"
    );
}

#[test]
fn the_public_certificate_persists_beside_the_lan_one_and_is_reused() {
    let dir = tempfile::tempdir().unwrap();
    let t = now();
    let lan = load_or_generate(
        Some(dir.path()),
        &[],
        &host(false),
        &RcgenGenerator,
        t,
        &test_trash(),
    )
    .unwrap();
    let first = load_or_generate_public(
        Some(dir.path()),
        &[],
        &host(false),
        None,
        &RcgenGenerator,
        t,
        &test_trash(),
    )
    .unwrap();
    assert!(first.persisted);
    assert!(dir.path().join(PUBLIC_CERT_FILE).is_file());
    assert!(dir.path().join(PUBLIC_KEY_FILE).is_file());
    assert_ne!(first.cert.fingerprint(), lan.cert.fingerprint());
    let second = load_or_generate_public(
        Some(dir.path()),
        &[],
        &host(false),
        None,
        &RcgenGenerator,
        t,
        &test_trash(),
    )
    .unwrap();
    assert_eq!(second.regenerated, None);
    assert_eq!(second.cert.fingerprint(), first.cert.fingerprint());
}

#[test]
fn a_new_interface_address_never_replaces_the_public_certificate() {
    let dir = tempfile::tempdir().unwrap();
    let t = now();
    let first = load_or_generate_public(
        Some(dir.path()),
        &[],
        &host(false),
        None,
        &RcgenGenerator,
        t,
        &test_trash(),
    )
    .unwrap();
    let mut changed = host(false);
    changed.interface_ips.push(ip("10.9.9.9"));
    let second = load_or_generate_public(
        Some(dir.path()),
        &[],
        &changed,
        None,
        &RcgenGenerator,
        t,
        &test_trash(),
    )
    .unwrap();
    assert_eq!(second.regenerated, None);
    assert!(second.drift.is_empty());
    assert_eq!(second.cert.fingerprint(), first.cert.fingerprint());
}

#[test]
fn the_public_certificate_follows_the_reuse_and_regeneration_rules() {
    let dir = tempfile::tempdir().unwrap();
    let t = now();
    let boot = |hostnames: &[String], sources: &SanSources, at: OffsetDateTime| {
        load_or_generate_public(
            Some(dir.path()),
            hostnames,
            sources,
            None,
            &RcgenGenerator,
            at,
            &test_trash(),
        )
        .unwrap()
    };
    let first = boot(&[], &host(false), t);

    // A newly configured tls_hostnames entry.
    let named = vec!["game.example".to_string()];
    let with_name = boot(&named, &host(false), t);
    assert_eq!(
        with_name.regenerated,
        Some(RegenReason::MissingHostname("game.example".into()))
    );
    assert_ne!(with_name.cert.fingerprint(), first.cert.fingerprint());
    assert_eq!(boot(&named, &host(false), t).regenerated, None);

    // The machine-hostname opt-in applies to this certificate too, in both directions.
    let opted_in = boot(&named, &host(true), t);
    assert_eq!(
        opted_in.regenerated,
        Some(RegenReason::MachineHostnameMissing)
    );
    assert!(opted_in
        .cert
        .san_entries()
        .contains(&"alice-laptop.local".to_string()));
    let opted_out = boot(&named, &host(false), t);
    assert_eq!(
        opted_out.regenerated,
        Some(RegenReason::MachineHostnameListed)
    );
    assert!(opted_out
        .cert
        .san_entries()
        .iter()
        .all(|e| !e.contains("alice")));

    // Expiry.
    let renewed = boot(&named, &host(false), t + Duration::days(380));
    assert_eq!(renewed.regenerated, Some(RegenReason::NearExpiry));
}

#[test]
fn force_regenerate_replaces_the_public_certificate_unconditionally() {
    let dir = tempfile::tempdir().unwrap();
    let t = now();
    let first = load_or_generate_public(
        Some(dir.path()),
        &[],
        &host(false),
        None,
        &RcgenGenerator,
        t,
        &test_trash(),
    )
    .unwrap();
    let forced = force_regenerate_public(
        dir.path(),
        &[],
        &host(false),
        None,
        &RcgenGenerator,
        t,
        &test_trash(),
    )
    .unwrap();
    assert_ne!(forced.fingerprint(), first.cert.fingerprint());
    assert!(forced
        .san_entries()
        .iter()
        .all(|e| e != "192.0.2.20" && e != "198.51.100.50"));
}

#[test]
fn only_a_known_external_address_the_certificate_lacks_is_a_reason() {
    let t = now();
    let cert = load_or_generate_public(
        None,
        &[],
        &host(false),
        Some(ip("203.0.113.9")),
        &RcgenGenerator,
        t,
        &test_trash(),
    )
    .unwrap()
    .cert;
    let at = t.unix_timestamp();
    let sources = host(false);
    assert_eq!(
        regen_reason_public(&cert, &[], &sources, Some(ip("203.0.113.9")), at),
        None
    );
    assert_eq!(
        regen_reason_public(&cert, &[], &sources, Some(ip("203.0.113.10")), at),
        Some(RegenReason::MissingExternalAddress(ip("203.0.113.10")))
    );
    assert_eq!(
        regen_reason_public(&cert, &[], &sources, None, at),
        None,
        "an address that is no longer known is not a reason to replace the certificate"
    );
}
