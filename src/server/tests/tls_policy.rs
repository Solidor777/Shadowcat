//! Certificate policy, selection, PEM handling, key-file protection and the degradation ladder.
//!
//! The tests are pure over the public `shadowcat::tls` API; the real-socket behaviour lives in the
//! `tls_listener` and `tls_http` tests.

use std::net::IpAddr;
use std::sync::Arc;

use shadowcat::config::{AcmeChallenge, Config, TlsMode};
use shadowcat::tls::pem::PemPaths;
use shadowcat::tls::resolver::{LoadedCert, PeerScope, Snapshot};
use shadowcat::tls::selfsigned::{
    drifted_addresses, eligible_interface_ips, generate, is_eligible_ip, is_virtual_adapter_name,
    load_or_generate, regen_reason, CertGenerator, Generated, InterfaceInfo, RcgenGenerator,
    RegenReason, SanSet, SanSources, CERT_FILE, KEY_FILE,
};
use shadowcat::tls::{Selected, TlsError, TlsRuntime, TlsSource};
use time::{Duration, OffsetDateTime};

fn now() -> OffsetDateTime {
    OffsetDateTime::now_utc()
}

/// A trash that removes outright: the policy tests never inspect trashed files.
fn test_trash() -> shadowcat::trash::Trash {
    shadowcat::trash::Trash::new(
        std::env::temp_dir().join("shadowcat-tls-policy-trash"),
        shadowcat::trash::TrashRetention::Disabled,
    )
}

fn sans(names: &[&str]) -> SanSet {
    SanSet {
        dns: names.iter().map(|s| s.to_string()).collect(),
        ips: vec![],
    }
}

fn cert_for(names: &[&str], at: OffsetDateTime) -> Arc<LoadedCert> {
    Arc::new(generate(&sans(names), at).unwrap().loaded)
}

fn ip(s: &str) -> IpAddr {
    s.parse().unwrap()
}

/// The generator seam lets the ladder's "generation failed" rung run without any production-only
/// hook.
struct FailingGenerator;

impl CertGenerator for FailingGenerator {
    fn generate(&self, _sans: &SanSet, _now: OffsetDateTime) -> Result<Generated, TlsError> {
        Err(TlsError::Generate("forced generation failure".into()))
    }
}

fn memory_config() -> Config {
    Config {
        db: "sqlite::memory:".into(),
        ..Config::default()
    }
}

fn write_pem(dir: &std::path::Path, g: &Generated) -> PemPaths {
    let paths = PemPaths {
        cert: dir.join("operator-cert.pem"),
        key: dir.join("operator-key.pem"),
    };
    std::fs::write(&paths.cert, &g.cert_pem).unwrap();
    std::fs::write(&paths.key, &g.key_pem).unwrap();
    paths
}

// ---------------------------------------------------------------------------------------------
// Self-signed attributes
// ---------------------------------------------------------------------------------------------

#[test]
fn self_signed_leaf_carries_the_specified_attributes() {
    let at = now();
    let sans = SanSet {
        dns: vec!["game.example".into()],
        ips: vec![ip("192.0.2.7")],
    };
    let g = generate(&sans, at).unwrap();
    let (_, cert) = x509_parser::parse_x509_certificate(g.cert_der.as_ref()).unwrap();

    // Server-auth EKU and digital-signature key usage, and an explicit CA:false.
    let eku = cert
        .extended_key_usage()
        .unwrap()
        .expect("EKU present")
        .value;
    assert!(eku.server_auth);
    assert!(!eku.client_auth);
    let ku = cert.key_usage().unwrap().expect("KU present").value;
    assert!(ku.digital_signature());
    assert!(!ku.key_cert_sign());
    let bc = cert
        .basic_constraints()
        .unwrap()
        .expect("basicConstraints present")
        .value;
    assert!(!bc.ca);

    // Validity: about one day back, about 397 days forward.
    let nb = cert.validity().not_before.timestamp();
    let na = cert.validity().not_after.timestamp();
    let t = at.unix_timestamp();
    assert!(
        (nb - (t - 86_400)).abs() <= 2,
        "notBefore {nb} vs {}",
        t - 86_400
    );
    assert!(
        (na - (t + 397 * 86_400)).abs() <= 2,
        "notAfter {na} vs {}",
        t + 397 * 86_400
    );

    // Subject CN and the key algorithm (ECDSA P-256).
    let cn = cert
        .subject()
        .iter_common_name()
        .next()
        .and_then(|a| a.as_str().ok())
        .unwrap();
    assert_eq!(cn, "Shadowcat server");
    let spki = cert.public_key();
    assert_eq!(spki.algorithm.algorithm.to_id_string(), "1.2.840.10045.2.1");
    let curve = spki
        .algorithm
        .parameters
        .as_ref()
        .and_then(|p| p.as_oid().ok())
        .map(|o| o.to_id_string());
    assert_eq!(curve.as_deref(), Some("1.2.840.10045.3.1.7"));

    // SANs: the assembled set and the base loopback names.
    let entries = g.loaded.san_entries();
    for want in ["localhost", "game.example", "192.0.2.7", "127.0.0.1", "::1"] {
        assert!(
            entries.contains(&want.to_string()),
            "missing {want}: {entries:?}"
        );
    }
}

#[test]
fn a_dns_entry_rcgen_rejects_is_skipped_not_fatal() {
    let g = generate(&sans(&["caf\u{e9}.example", "ok.example"]), now()).unwrap();
    let entries = g.loaded.san_entries();
    assert!(entries.contains(&"ok.example".to_string()));
    assert!(!entries.iter().any(|e| e.contains('\u{e9}')));
}

// ---------------------------------------------------------------------------------------------
// Reuse and regeneration
// ---------------------------------------------------------------------------------------------

#[test]
fn stored_certificate_is_reused_across_boots() {
    let dir = tempfile::tempdir().unwrap();
    let t = now();
    let first = load_or_generate(
        Some(dir.path()),
        &[],
        &SanSources::default(),
        &RcgenGenerator,
        t,
        &test_trash(),
    )
    .unwrap();
    assert!(first.persisted && first.regenerated.is_none());
    let second = load_or_generate(
        Some(dir.path()),
        &[],
        &SanSources::default(),
        &RcgenGenerator,
        t,
        &test_trash(),
    )
    .unwrap();
    assert!(second.persisted && second.regenerated.is_none());
    assert_eq!(first.cert.fingerprint(), second.cert.fingerprint());
}

#[test]
fn a_new_interface_address_does_not_regenerate_but_is_reported_as_drift() {
    let dir = tempfile::tempdir().unwrap();
    let t = now();
    let boot1 = load_or_generate(
        Some(dir.path()),
        &[],
        &SanSources::default(),
        &RcgenGenerator,
        t,
        &test_trash(),
    )
    .unwrap();
    let with_new_lan = SanSources {
        interface_ips: vec![ip("192.168.50.9")],
        ..SanSources::default()
    };
    let boot2 = load_or_generate(
        Some(dir.path()),
        &[],
        &with_new_lan,
        &RcgenGenerator,
        t,
        &test_trash(),
    )
    .unwrap();
    assert_eq!(boot1.cert.fingerprint(), boot2.cert.fingerprint());
    assert_eq!(boot2.drift, vec![ip("192.168.50.9")]);
    assert_eq!(
        drifted_addresses(&boot2.cert, &[ip("192.168.50.9")]),
        vec![ip("192.168.50.9")]
    );
}

#[test]
fn a_missing_configured_hostname_regenerates() {
    let dir = tempfile::tempdir().unwrap();
    let t = now();
    let boot1 = load_or_generate(
        Some(dir.path()),
        &[],
        &SanSources::default(),
        &RcgenGenerator,
        t,
        &test_trash(),
    )
    .unwrap();
    let hosts = vec!["table.example".to_string()];
    let boot2 = load_or_generate(
        Some(dir.path()),
        &hosts,
        &SanSources::default(),
        &RcgenGenerator,
        t,
        &test_trash(),
    )
    .unwrap();
    assert_eq!(
        boot2.regenerated,
        Some(RegenReason::MissingHostname("table.example".into()))
    );
    assert_ne!(boot1.cert.fingerprint(), boot2.cert.fingerprint());
    assert!(boot2.cert.covers_name("table.example"));
    // Now present: a third boot reuses.
    let boot3 = load_or_generate(
        Some(dir.path()),
        &hosts,
        &SanSources::default(),
        &RcgenGenerator,
        t,
        &test_trash(),
    )
    .unwrap();
    assert_eq!(boot3.cert.fingerprint(), boot2.cert.fingerprint());
}

#[test]
fn near_expiry_regenerates_by_the_injected_clock() {
    let dir = tempfile::tempdir().unwrap();
    let t = now();
    let boot1 = load_or_generate(
        Some(dir.path()),
        &[],
        &SanSources::default(),
        &RcgenGenerator,
        t,
        &test_trash(),
    )
    .unwrap();
    // 366 days later the 397-day leaf has 31 days left: still reused.
    let later = t + Duration::days(366);
    let reuse = load_or_generate(
        Some(dir.path()),
        &[],
        &SanSources::default(),
        &RcgenGenerator,
        later,
        &test_trash(),
    )
    .unwrap();
    assert!(reuse.regenerated.is_none());
    // 368 days later it has 29 days left: regenerated.
    let near = t + Duration::days(368);
    let renewed = load_or_generate(
        Some(dir.path()),
        &[],
        &SanSources::default(),
        &RcgenGenerator,
        near,
        &test_trash(),
    )
    .unwrap();
    assert_eq!(renewed.regenerated, Some(RegenReason::NearExpiry));
    assert_ne!(boot1.cert.fingerprint(), renewed.cert.fingerprint());
    assert_eq!(
        regen_reason(
            &boot1.cert,
            &[],
            &SanSources::default(),
            near.unix_timestamp()
        ),
        Some(RegenReason::NearExpiry)
    );
}

#[test]
fn unparsable_or_half_stored_files_regenerate() {
    let dir = tempfile::tempdir().unwrap();
    let t = now();
    let first = load_or_generate(
        Some(dir.path()),
        &[],
        &SanSources::default(),
        &RcgenGenerator,
        t,
        &test_trash(),
    )
    .unwrap();
    std::fs::write(dir.path().join(CERT_FILE), "garbage").unwrap();
    let second = load_or_generate(
        Some(dir.path()),
        &[],
        &SanSources::default(),
        &RcgenGenerator,
        t,
        &test_trash(),
    )
    .unwrap();
    assert_eq!(second.regenerated, Some(RegenReason::Unusable));
    assert_ne!(first.cert.fingerprint(), second.cert.fingerprint());
    // Only the key file left.
    std::fs::rename(dir.path().join(CERT_FILE), dir.path().join("cert.aside")).unwrap();
    let third = load_or_generate(
        Some(dir.path()),
        &[],
        &SanSources::default(),
        &RcgenGenerator,
        t,
        &test_trash(),
    )
    .unwrap();
    assert!(third.persisted);
    assert!(dir.path().join(CERT_FILE).exists() && dir.path().join(KEY_FILE).exists());
}

#[test]
fn no_directory_serves_an_in_memory_certificate() {
    let out = load_or_generate(
        None,
        &[],
        &SanSources::default(),
        &RcgenGenerator,
        now(),
        &test_trash(),
    )
    .unwrap();
    assert!(!out.persisted);
}

// ---------------------------------------------------------------------------------------------
// Interface classifier
// ---------------------------------------------------------------------------------------------

#[test]
fn ip_eligibility_table() {
    for (addr, want) in [
        ("192.168.1.20", true),
        ("10.0.0.5", true),
        ("172.16.4.4", true),
        ("100.64.0.9", true),
        ("8.8.4.4", true),
        ("127.0.0.1", false),
        ("169.254.10.1", false),
        ("0.0.0.0", false),
        ("224.0.0.1", false),
        ("255.255.255.255", false),
        ("2001:db8::1", true),
        ("fd12:3456::1", true),
        ("fe80::1", false),
        ("::1", false),
        ("::", false),
        ("ff02::1", false),
    ] {
        assert_eq!(is_eligible_ip(ip(addr)), want, "{addr}");
    }
}

#[test]
fn virtual_adapter_name_table() {
    for (name, want) in [
        ("docker0", true),
        ("br-1a2b3c", true),
        ("veth1234", true),
        ("virbr0", true),
        ("vmnet8", true),
        ("vboxnet0", true),
        ("vEthernet (Default Switch)", true),
        ("VETHERNET (WSL)", true),
        ("eth0", false),
        ("en0", false),
        ("Wi-Fi", false),
        ("Ethernet", false),
        ("wlan0", false),
    ] {
        assert_eq!(is_virtual_adapter_name(name), want, "{name}");
    }
}

#[test]
fn interface_selection_filters_virtual_adapters_and_dedupes() {
    let infos = vec![
        InterfaceInfo {
            names: vec!["eth0".into()],
            addrs: vec![ip("192.168.1.20"), ip("fe80::1"), ip("192.168.1.20")],
        },
        InterfaceInfo {
            names: vec!["{GUID}".into(), "vEthernet (WSL)".into()],
            addrs: vec![ip("172.29.0.1")],
        },
        InterfaceInfo {
            names: vec!["docker0".into()],
            addrs: vec![ip("172.17.0.1")],
        },
        InterfaceInfo {
            names: vec!["lo".into()],
            addrs: vec![ip("127.0.0.1")],
        },
    ];
    assert_eq!(eligible_interface_ips(&infos), vec![ip("192.168.1.20")]);
}

#[test]
fn san_assembly_skips_nothing_it_should_keep() {
    let sources = SanSources {
        machine_hostname: Some("Table-PC".into()),
        advertise_hostname: true,
        interface_ips: vec![ip("192.168.1.20")],
    };
    let set = SanSet::assemble(
        &["Game.Example".to_string(), "10.1.1.1".to_string()],
        &sources,
    );
    assert!(set.dns.contains(&"localhost".to_string()));
    assert!(set.dns.contains(&"table-pc".to_string()));
    assert!(set.dns.contains(&"table-pc.local".to_string()));
    assert!(set.dns.contains(&"game.example".to_string()));
    assert!(set.ips.contains(&ip("10.1.1.1")));
    assert!(set.ips.contains(&ip("192.168.1.20")));
    assert!(set.ips.contains(&ip("127.0.0.1")) && set.ips.contains(&ip("::1")));
}

fn host_sources(advertise: bool) -> SanSources {
    SanSources {
        machine_hostname: Some("Alice-Laptop".into()),
        advertise_hostname: advertise,
        interface_ips: vec![ip("192.0.2.20")],
    }
}

#[test]
fn the_machine_hostname_is_never_listed_unless_opted_in() {
    let off = SanSet::assemble(&[], &host_sources(false));
    assert!(
        off.dns.iter().all(|d| !d.contains("alice")),
        "{:?}",
        off.dns
    );
    assert!(off.ips.contains(&ip("192.0.2.20")), "interface IPs stay");
    let generated = generate(&off, now()).unwrap();
    assert!(generated
        .loaded
        .san_entries()
        .iter()
        .all(|e| !e.contains("alice")));
}

#[test]
fn a_persisted_certificate_listing_the_hostname_is_replaced_when_the_opt_in_is_off() {
    let dir = tempfile::tempdir().unwrap();
    let t = now();
    let opted_in = load_or_generate(
        Some(dir.path()),
        &[],
        &host_sources(true),
        &RcgenGenerator,
        t,
        &test_trash(),
    )
    .unwrap();
    assert!(opted_in
        .cert
        .san_entries()
        .contains(&"alice-laptop.local".to_string()));
    let off = load_or_generate(
        Some(dir.path()),
        &[],
        &host_sources(false),
        &RcgenGenerator,
        t,
        &test_trash(),
    )
    .unwrap();
    assert_eq!(off.regenerated, Some(RegenReason::MachineHostnameListed));
    assert!(off.cert.san_entries().iter().all(|e| !e.contains("alice")));
    // Stable afterwards: no regeneration loop.
    let again = load_or_generate(
        Some(dir.path()),
        &[],
        &host_sources(false),
        &RcgenGenerator,
        t,
        &test_trash(),
    )
    .unwrap();
    assert_eq!(again.regenerated, None);
    assert_eq!(again.cert.fingerprint(), off.cert.fingerprint());
}

#[test]
fn turning_the_opt_in_on_regenerates_once_and_then_is_stable() {
    let dir = tempfile::tempdir().unwrap();
    let t = now();
    load_or_generate(
        Some(dir.path()),
        &[],
        &host_sources(false),
        &RcgenGenerator,
        t,
        &test_trash(),
    )
    .unwrap();
    let on = load_or_generate(
        Some(dir.path()),
        &[],
        &host_sources(true),
        &RcgenGenerator,
        t,
        &test_trash(),
    )
    .unwrap();
    assert_eq!(on.regenerated, Some(RegenReason::MachineHostnameMissing));
    let again = load_or_generate(
        Some(dir.path()),
        &[],
        &host_sources(true),
        &RcgenGenerator,
        t,
        &test_trash(),
    )
    .unwrap();
    assert_eq!(again.regenerated, None);
}

#[test]
fn a_hostname_the_operator_named_in_tls_hostnames_is_not_treated_as_listed() {
    let dir = tempfile::tempdir().unwrap();
    let t = now();
    let named = vec!["Alice-Laptop".to_string()];
    let first = load_or_generate(
        Some(dir.path()),
        &named,
        &host_sources(false),
        &RcgenGenerator,
        t,
        &test_trash(),
    )
    .unwrap();
    assert!(first
        .cert
        .san_entries()
        .contains(&"alice-laptop".to_string()));
    let second = load_or_generate(
        Some(dir.path()),
        &named,
        &host_sources(false),
        &RcgenGenerator,
        t,
        &test_trash(),
    )
    .unwrap();
    assert_eq!(
        second.regenerated, None,
        "the operator's own entry is theirs"
    );
}

#[test]
fn an_unusable_machine_hostname_neither_appears_nor_forces_regeneration() {
    let dir = tempfile::tempdir().unwrap();
    let t = now();
    for host in ["localhost", "caf\u{e9}-pc", "192.0.2.5", ""] {
        let sources = SanSources {
            machine_hostname: Some(host.into()),
            advertise_hostname: true,
            ..SanSources::default()
        };
        let a = load_or_generate(
            Some(dir.path()),
            &[],
            &sources,
            &RcgenGenerator,
            t,
            &test_trash(),
        )
        .unwrap();
        let b = load_or_generate(
            Some(dir.path()),
            &[],
            &sources,
            &RcgenGenerator,
            t,
            &test_trash(),
        )
        .unwrap();
        assert_eq!(b.regenerated, None, "{host:?}");
        assert!(a.cert.fingerprint() == b.cert.fingerprint(), "{host:?}");
    }
}

// ---------------------------------------------------------------------------------------------
// Key-file protection
// ---------------------------------------------------------------------------------------------

#[cfg(unix)]
#[test]
fn key_file_and_directory_are_owner_only_on_unix() {
    use std::os::unix::fs::PermissionsExt;
    let base = tempfile::tempdir().unwrap();
    let dir = base.path().join("tls");
    load_or_generate(
        Some(&dir),
        &[],
        &SanSources::default(),
        &RcgenGenerator,
        now(),
        &test_trash(),
    )
    .unwrap();
    let key_mode = std::fs::metadata(dir.join(KEY_FILE))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    let dir_mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
    assert_eq!(key_mode, 0o600);
    assert_eq!(dir_mode, 0o700);
}

#[cfg(windows)]
#[test]
fn key_file_dacl_is_protected_and_owner_only_on_windows() {
    let base = tempfile::tempdir().unwrap();
    let dir = base.path().join("tls");
    load_or_generate(
        Some(&dir),
        &[],
        &SanSources::default(),
        &RcgenGenerator,
        now(),
        &test_trash(),
    )
    .unwrap();
    for path in [dir.join(KEY_FILE), dir.clone()] {
        let sddl = shadowcat::tls::keyfile::dacl_sddl(&path).unwrap();
        // Protected (no inherited ACEs) with exactly one ACE, granting a user SID full control.
        assert!(
            sddl.starts_with("D:P") || sddl.starts_with("D:PAI"),
            "{path:?}: {sddl}"
        );
        assert_eq!(sddl.matches('(').count(), 1, "{path:?}: {sddl}");
        for broad in ["WD", "BU", "AU", "IU", "BG", "S-1-1-0"] {
            assert!(
                !sddl.contains(&format!(";{broad})")),
                "{path:?} grants a broad principal {broad}: {sddl}"
            );
        }
        assert!(sddl.contains(";FA;"), "{path:?}: {sddl}");
        // The principal is a user SID, or `LA`: SDDL writes the built-in Administrator account
        // (the account a hosted CI runner executes as) as its alias `LA` rather than as
        // `S-1-5-21-<domain>-500`. Source: Microsoft "SDDL for Device Objects" / "ACE strings",
        // alias table.
        assert!(
            sddl.contains("S-1-5-") || sddl.contains(";LA)"),
            "{path:?}: {sddl}"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Selection
// ---------------------------------------------------------------------------------------------

#[test]
fn select_table_over_sni_alpn_and_validity() {
    let t = now();
    let fallback = cert_for(&[], t);
    let primary = cert_for(&["game.example", "*.wild.example"], t);
    let far_future = primary.not_after() + 10;
    let snap = Snapshot::new(Some(primary), fallback, Default::default()).unwrap();

    let sel = |sni: Option<&str>| snap.select(sni, &[]);
    assert_eq!(sel(Some("game.example")), Selected::Primary);
    assert_eq!(sel(Some("GAME.example")), Selected::Primary);
    assert_eq!(sel(Some("a.wild.example")), Selected::Primary);
    assert_eq!(sel(Some("a.b.wild.example")), Selected::Fallback);
    assert_eq!(sel(Some("wild.example")), Selected::Fallback);
    assert_eq!(sel(Some("other.example")), Selected::Fallback);
    assert_eq!(sel(None), Selected::Fallback);
    // An IP literal in the SNI slot is never a name match.
    assert_eq!(sel(Some("127.0.0.1")), Selected::Fallback);
    // An expired primary routes to the fallback.
    assert_eq!(
        snap.select_at(Some("game.example"), &[], far_future),
        Selected::Fallback
    );
    // A snapshot with no primary never returns Primary.
    let bare = Snapshot::new(None, cert_for(&[], t), Default::default()).unwrap();
    assert_eq!(bare.select(Some("game.example"), &[]), Selected::Fallback);
}

#[test]
fn acme_tls_alpn_selects_the_challenge_certificate_for_its_sni() {
    use std::collections::BTreeMap;
    let t = now();
    let challenge = cert_for(&["game.example"], t);
    let mut map = BTreeMap::new();
    map.insert(
        "game.example".to_string(),
        challenge.certified_key().clone(),
    );
    let snap = Snapshot::new(None, cert_for(&[], t), map).unwrap();
    let acme: &[&[u8]] = &[b"acme-tls/1"];
    assert_eq!(snap.select(Some("game.example"), acme), Selected::Challenge);
    // No challenge for another name; ordinary ALPN never selects one.
    assert_eq!(snap.select(Some("other.example"), acme), Selected::Fallback);
    assert_eq!(
        snap.select(Some("game.example"), &[b"h2"]),
        Selected::Fallback
    );
    // The challenge configuration advertises only acme-tls/1; serving configs advertise h2 then
    // http/1.1.
    assert_eq!(
        snap.config_for(Selected::Challenge, PeerScope::Lan)
            .alpn_protocols,
        vec![b"acme-tls/1".to_vec()]
    );
    assert_eq!(
        snap.config_for(Selected::Fallback, PeerScope::Lan)
            .alpn_protocols,
        vec![b"h2".to_vec(), b"http/1.1".to_vec()]
    );
}

#[test]
fn hot_swap_installs_a_new_snapshot_and_never_mutates_one() {
    use shadowcat::tls::resolver::CertResolver;
    let t = now();
    let cell =
        CertResolver::new(Snapshot::new(None, cert_for(&[], t), Default::default()).unwrap());
    let before = cell.snapshot();
    let p = cert_for(&["game.example"], t);
    cell.update(|cur| cur.with_primary(Some(p.clone())))
        .unwrap();
    assert!(before.primary().is_none(), "the old snapshot is immutable");
    assert_eq!(
        cell.snapshot().primary().unwrap().fingerprint(),
        p.fingerprint()
    );
}

#[test]
fn concurrent_updates_never_lose_a_writers_change() {
    // Two writers each own one slot (primary / fallback). Every install derives from the current
    // snapshot, so the final state must hold BOTH writers' last values; a read-then-install pair
    // lets the later install resurrect the other slot's stale value.
    use shadowcat::tls::resolver::CertResolver;
    let t = now();
    let cell = Arc::new(CertResolver::new(
        Snapshot::new(None, cert_for(&[], t), Default::default()).unwrap(),
    ));
    let primaries: Vec<Arc<LoadedCert>> = (0..4).map(|_| cert_for(&["p.example"], t)).collect();
    let fallbacks: Vec<Arc<LoadedCert>> = (0..4).map(|_| cert_for(&[], t)).collect();
    let rounds = 300;
    let a = {
        let (cell, primaries) = (cell.clone(), primaries.clone());
        std::thread::spawn(move || {
            for i in 0..rounds {
                let p = primaries[i % 4].clone();
                cell.update(|cur| cur.with_primary(Some(p))).unwrap();
            }
        })
    };
    let b = {
        let (cell, fallbacks) = (cell.clone(), fallbacks.clone());
        std::thread::spawn(move || {
            for i in 0..rounds {
                let f = fallbacks[i % 4].clone();
                cell.update(|cur| cur.with_fallback(f)).unwrap();
            }
        })
    };
    a.join().unwrap();
    b.join().unwrap();
    let last = (rounds - 1) % 4;
    let snap = cell.snapshot();
    assert_eq!(
        snap.primary().unwrap().fingerprint(),
        primaries[last].fingerprint(),
        "the fallback writer discarded the primary writer's last install"
    );
    assert_eq!(
        snap.fallback().fingerprint(),
        fallbacks[last].fingerprint(),
        "the primary writer discarded the fallback writer's last install"
    );
}

// ---------------------------------------------------------------------------------------------
// Operator PEM
// ---------------------------------------------------------------------------------------------

#[test]
fn operator_pem_valid_mismatched_expired_garbage() {
    let dir = tempfile::tempdir().unwrap();
    let t = now();
    let good = generate(&sans(&["game.example"]), t).unwrap();
    let paths = write_pem(dir.path(), &good);
    let loaded = paths.load(t.unix_timestamp()).unwrap();
    assert!(loaded.covers_name("game.example"));

    // Certificate of one key with the private key of another.
    let other = generate(&sans(&["other.example"]), t).unwrap();
    std::fs::write(&paths.key, &other.key_pem).unwrap();
    assert!(matches!(
        paths.load(t.unix_timestamp()),
        Err(TlsError::Cert(_))
    ));

    // Expired: generated 500 days ago, so notAfter is 103 days in the past.
    let old = generate(&sans(&["game.example"]), t - Duration::days(500)).unwrap();
    let old_paths = write_pem(dir.path(), &old);
    assert!(matches!(
        old_paths.load(t.unix_timestamp()),
        Err(TlsError::Expired(_))
    ));

    // Garbage and missing files.
    std::fs::write(&paths.cert, "not a pem").unwrap();
    assert!(paths.load(t.unix_timestamp()).is_err());
    let missing = PemPaths {
        cert: dir.path().join("absent-cert.pem"),
        key: dir.path().join("absent-key.pem"),
    };
    assert!(missing.load(t.unix_timestamp()).is_err());
}

// ---------------------------------------------------------------------------------------------
// The ladder
// ---------------------------------------------------------------------------------------------

fn start(config: &Config) -> Arc<TlsRuntime> {
    TlsRuntime::start_with(
        config,
        Arc::new(RcgenGenerator),
        SanSources::default(),
        now(),
    )
}

#[test]
fn defaults_serve_an_in_memory_self_signed_certificate() {
    let rt = start(&memory_config());
    let status = rt.status();
    assert!(status.enabled);
    assert_eq!(status.source, Some(TlsSource::SelfSigned));
    assert_eq!(status.alpn, vec!["h2".to_string(), "http/1.1".to_string()]);
    let admin = rt.admin_status();
    assert_eq!(admin.fingerprint, admin.fallback_fingerprint);
    assert!(admin.degraded_from.is_none());
}

#[test]
fn tls_off_has_no_resolver() {
    let rt = start(&Config {
        tls: TlsMode::Off,
        ..memory_config()
    });
    assert!(rt.resolver().is_none());
    assert!(!rt.status().enabled && rt.status().source.is_none());
}

#[test]
fn valid_operator_pem_becomes_the_primary_source() {
    let dir = tempfile::tempdir().unwrap();
    let g = generate(&sans(&["game.example", "*.wild.example"]), now()).unwrap();
    let paths = write_pem(dir.path(), &g);
    let rt = start(&Config {
        tls_cert: Some(paths.cert.to_string_lossy().into_owned()),
        tls_key: Some(paths.key.to_string_lossy().into_owned()),
        bind: "0.0.0.0:31234".into(),
        ..memory_config()
    });
    let status = rt.status();
    assert_eq!(status.source, Some(TlsSource::Operator));
    // Published: non-wildcard DNS SANs of the loaded leaf on the bind port; never the machine
    // hostname, never localhost-derived names beyond what the leaf carries.
    assert!(status
        .https_origins
        .contains(&"https://game.example:31234".to_string()));
    assert!(!status.https_origins.iter().any(|o| o.contains('*')));
    let admin = rt.admin_status();
    assert_eq!(admin.fingerprint.as_deref(), Some(g.loaded.fingerprint()));
    assert_ne!(admin.fingerprint, admin.fallback_fingerprint);
}

#[test]
fn expired_pem_at_boot_degrades_to_self_signed_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let old = generate(&sans(&["game.example"]), now() - Duration::days(500)).unwrap();
    let paths = write_pem(dir.path(), &old);
    let rt = start(&Config {
        tls_cert: Some(paths.cert.to_string_lossy().into_owned()),
        tls_key: Some(paths.key.to_string_lossy().into_owned()),
        ..memory_config()
    });
    assert!(rt.status().enabled);
    assert_eq!(rt.status().source, Some(TlsSource::SelfSigned));
    let admin = rt.admin_status();
    assert_eq!(admin.degraded_from, Some(TlsSource::Operator));
    assert!(admin.last_error.is_some());
}

#[test]
fn garbage_pem_at_boot_degrades() {
    let dir = tempfile::tempdir().unwrap();
    let cert = dir.path().join("c.pem");
    let key = dir.path().join("k.pem");
    std::fs::write(&cert, "garbage").unwrap();
    std::fs::write(&key, "garbage").unwrap();
    let rt = start(&Config {
        tls_cert: Some(cert.to_string_lossy().into_owned()),
        tls_key: Some(key.to_string_lossy().into_owned()),
        ..memory_config()
    });
    assert_eq!(rt.status().source, Some(TlsSource::SelfSigned));
    assert_eq!(rt.admin_status().degraded_from, Some(TlsSource::Operator));
}

#[test]
fn unwritable_tls_dir_falls_back_to_an_in_memory_certificate() {
    // A path UNDER a regular file cannot be created as a directory on any platform.
    let dir = tempfile::tempdir().unwrap();
    let blocker = dir.path().join("blocker");
    std::fs::write(&blocker, "x").unwrap();
    let rt = start(&Config {
        tls_dir: Some(blocker.join("tls").to_string_lossy().into_owned()),
        ..memory_config()
    });
    assert!(
        rt.status().enabled,
        "TLS stays up on an in-memory certificate"
    );
    assert_eq!(rt.status().source, Some(TlsSource::SelfSigned));
    assert!(rt.admin_status().last_error.is_some());
}

#[test]
fn forced_generation_failure_disables_tls_only() {
    let rt = TlsRuntime::start_with(
        &memory_config(),
        Arc::new(FailingGenerator),
        SanSources::default(),
        now(),
    );
    assert!(rt.resolver().is_none());
    assert!(!rt.status().enabled);
    let admin = rt.admin_status();
    assert!(admin
        .last_error
        .unwrap()
        .contains("forced generation failure"));
}

#[test]
fn unreadable_acme_ca_root_degrades_acme_and_reports_it() {
    let tls_dir = tempfile::tempdir().unwrap();
    let rt = start(&Config {
        acme_domains: vec!["game.example".into()],
        acme_terms_agreed: true,
        acme_ca_root: Some("does-not-exist.pem".into()),
        tls_dir: Some(tls_dir.path().to_string_lossy().into_owned()),
        ..memory_config()
    });
    assert!(rt.status().enabled);
    assert_eq!(rt.status().source, Some(TlsSource::SelfSigned));
    let admin = rt.admin_status();
    assert_eq!(admin.degraded_from, Some(TlsSource::Acme));
    assert!(admin.last_error.unwrap().contains("acme_ca_root"));
}

#[test]
fn https_origins_come_only_from_explicit_configuration() {
    // Nothing configured: nothing published (the machine hostname never is).
    let plain = start(&memory_config());
    assert!(plain.status().https_origins.is_empty());
    // public_url wins and is published verbatim (trailing slash trimmed).
    let public = start(&Config {
        public_url: Some("https://vtt.example/".into()),
        tls_hostnames: vec!["ignored.example".into()],
        ..memory_config()
    });
    assert_eq!(
        public.status().https_origins,
        vec!["https://vtt.example".to_string()]
    );
    // tls_hostnames: DNS entries only, on the bind port; IP and wildcard entries are not origins.
    let hosts = start(&Config {
        tls_hostnames: vec!["a.example".into(), "10.0.0.5".into(), "*.b.example".into()],
        bind: "0.0.0.0:31111".into(),
        ..memory_config()
    });
    assert_eq!(
        hosts.status().https_origins,
        vec!["https://a.example:31111".to_string()]
    );
    // ACME domains: 443 for tls-alpn-01 (no port), the bind port for http-01.
    let tls_dir = tempfile::tempdir().unwrap();
    let acme = |challenge| {
        start(&Config {
            acme_domains: vec!["game.example".into()],
            acme_terms_agreed: true,
            acme_challenge: challenge,
            tls_dir: Some(tls_dir.path().to_string_lossy().into_owned()),
            bind: "0.0.0.0:31111".into(),
            ..memory_config()
        })
    };
    assert_eq!(
        acme(AcmeChallenge::TlsAlpn01).status().https_origins,
        vec!["https://game.example".to_string()]
    );
    assert_eq!(
        acme(AcmeChallenge::Http01).status().https_origins,
        vec!["https://game.example:31111".to_string()]
    );
}

// ---------------------------------------------------------------------------------------------
// Maintenance
// ---------------------------------------------------------------------------------------------

#[test]
fn pem_reload_swaps_on_change_and_keeps_the_previous_certificate_on_failure() {
    let dir = tempfile::tempdir().unwrap();
    let t = now();
    let first = generate(&sans(&["game.example"]), t).unwrap();
    let paths = write_pem(dir.path(), &first);
    let rt = TlsRuntime::start_with(
        &Config {
            tls_cert: Some(paths.cert.to_string_lossy().into_owned()),
            tls_key: Some(paths.key.to_string_lossy().into_owned()),
            ..memory_config()
        },
        Arc::new(RcgenGenerator),
        SanSources::default(),
        t,
    );
    let fp = |rt: &TlsRuntime| rt.admin_status().fingerprint.unwrap();
    assert_eq!(fp(&rt), first.loaded.fingerprint());

    // A new certificate lands with a later mtime: reloaded.
    let second = generate(&sans(&["game.example"]), t).unwrap();
    std::fs::write(&paths.cert, &second.cert_pem).unwrap();
    std::fs::write(&paths.key, &second.key_pem).unwrap();
    bump_mtime(&paths.cert, 10);
    bump_mtime(&paths.key, 10);
    rt.maintenance_tick(t + Duration::seconds(60));
    assert_eq!(fp(&rt), second.loaded.fingerprint());
    assert!(rt.admin_status().degraded_from.is_none());

    // A broken replacement keeps the previous good certificate and records the error.
    std::fs::write(&paths.cert, "garbage").unwrap();
    bump_mtime(&paths.cert, 20);
    rt.maintenance_tick(t + Duration::seconds(120));
    assert_eq!(fp(&rt), second.loaded.fingerprint());
    assert!(rt.admin_status().last_error.is_some());
}

/// Sets `path`'s modification time `secs` seconds past the epoch-relative base, so a test never
/// depends on filesystem timestamp granularity.
fn bump_mtime(path: &std::path::Path, secs: u64) {
    let f = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    let when =
        std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000 + secs);
    f.set_modified(when).unwrap();
}

#[test]
fn an_expired_operator_certificate_routes_to_the_fallback_without_a_reload() {
    // Primary valid at load, then the clock moves past its notAfter: selection (not a reload)
    // is what protects callers.
    let dir = tempfile::tempdir().unwrap();
    let t = now();
    let g = generate(&sans(&["game.example"]), t).unwrap();
    let paths = write_pem(dir.path(), &g);
    let rt = TlsRuntime::start_with(
        &Config {
            tls_cert: Some(paths.cert.to_string_lossy().into_owned()),
            tls_key: Some(paths.key.to_string_lossy().into_owned()),
            ..memory_config()
        },
        Arc::new(RcgenGenerator),
        SanSources::default(),
        t,
    );
    let snap = rt.resolver().unwrap().snapshot();
    let after = g.loaded.not_after() + 5;
    assert_eq!(
        snap.select_at(Some("game.example"), &[], after),
        Selected::Fallback
    );
}

#[test]
fn daily_check_regenerates_a_near_expiry_self_signed_certificate_and_swaps_it() {
    let dir = tempfile::tempdir().unwrap();
    let t = now();
    let rt = TlsRuntime::start_with(
        &Config {
            tls_dir: Some(dir.path().to_string_lossy().into_owned()),
            ..memory_config()
        },
        Arc::new(RcgenGenerator),
        SanSources::default(),
        t,
    );
    let before = rt.admin_status().fallback_fingerprint.unwrap();
    // Within a day of the last check nothing runs, whatever the clock says.
    rt.maintenance_tick(t + Duration::hours(1));
    assert_eq!(rt.admin_status().fallback_fingerprint.unwrap(), before);
    // A day later, inside the renewal window: regenerated and hot-swapped.
    rt.maintenance_tick(t + Duration::days(370));
    let after = rt.admin_status().fallback_fingerprint.unwrap();
    assert_ne!(before, after);
}

// ---------------------------------------------------------------------------------------------
// Effective status, published origins, diagnostics and logging
// ---------------------------------------------------------------------------------------------

fn operator_runtime(
    dir: &std::path::Path,
    dns: &[&str],
    extra: impl FnOnce(&mut Config),
    t: OffsetDateTime,
) -> (Arc<TlsRuntime>, Generated) {
    let g = generate(&sans(dns), t).unwrap();
    let paths = write_pem(dir, &g);
    let mut config = Config {
        tls_cert: Some(paths.cert.to_string_lossy().into_owned()),
        tls_key: Some(paths.key.to_string_lossy().into_owned()),
        bind: "0.0.0.0:31234".into(),
        ..memory_config()
    };
    extra(&mut config);
    let rt = TlsRuntime::start_with(&config, Arc::new(RcgenGenerator), SanSources::default(), t);
    (rt, g)
}

#[test]
fn status_reports_the_certificate_actually_served_once_the_operator_certificate_expires() {
    let dir = tempfile::tempdir().unwrap();
    let t = now();
    let (rt, g) = operator_runtime(dir.path(), &["game.example"], |_| {}, t);
    let live = rt.status_at(t);
    assert_eq!(live.source, Some(TlsSource::Operator));
    assert_eq!(
        rt.admin_status_at(t).fingerprint.as_deref(),
        Some(g.loaded.fingerprint())
    );
    let later = OffsetDateTime::from_unix_timestamp(g.loaded.not_after() + 5).unwrap();
    let expired = rt.status_at(later);
    assert_eq!(expired.source, Some(TlsSource::SelfSigned));
    assert!(
        !expired
            .https_origins
            .contains(&"https://game.example:31234".to_string()),
        "an expired certificate's names are no longer published"
    );
    let admin = rt.admin_status_at(later);
    assert_eq!(admin.fingerprint, admin.fallback_fingerprint);
    assert_ne!(admin.fingerprint.as_deref(), Some(g.loaded.fingerprint()));
}

#[test]
fn published_origins_are_the_union_of_configured_names_and_the_operator_certificates() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, _g) = operator_runtime(
        dir.path(),
        &["game.example", "*.wild.example"],
        |c| c.tls_hostnames = vec!["table.example".into(), "192.0.2.7".into()],
        now(),
    );
    let origins = rt.status().https_origins;
    assert!(
        origins.contains(&"https://table.example:31234".to_string()),
        "{origins:?}"
    );
    assert!(
        origins.contains(&"https://game.example:31234".to_string()),
        "the certificate's own DNS names are published beside tls_hostnames: {origins:?}"
    );
    assert!(origins
        .iter()
        .all(|o| !o.contains('*') && !o.contains("192.0.2.7")));
    assert_eq!(
        origins.len(),
        origins
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        "no duplicates"
    );
}

#[test]
fn configured_dns_names_are_published_lowercased() {
    let rt = start(&Config {
        tls_hostnames: vec!["Table.Example".into()],
        bind: "0.0.0.0:31111".into(),
        ..memory_config()
    });
    assert_eq!(
        rt.status().https_origins,
        vec!["https://table.example:31111".to_string()]
    );
}

#[test]
fn a_public_url_that_is_not_an_origin_does_not_suppress_the_operator_certificates_names() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, _g) = operator_runtime(
        dir.path(),
        &["game.example"],
        |c| c.public_url = Some("https://vtt.example/not-an-origin".into()),
        now(),
    );
    let origins = rt.status().https_origins;
    assert!(
        origins.contains(&"https://game.example:31234".to_string()),
        "a rejected public_url is not a published public_url: {origins:?}"
    );
    assert!(
        !origins.iter().any(|o| o.contains("vtt.example")),
        "{origins:?}"
    );
}

#[test]
fn a_public_url_is_published_as_its_ascii_origin() {
    let rt = start(&Config {
        public_url: Some("https://Play.Example:443/".into()),
        ..memory_config()
    });
    assert_eq!(
        rt.status().https_origins,
        vec!["https://play.example".to_string()]
    );
}

#[test]
fn a_public_url_is_published_alone() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, _g) = operator_runtime(
        dir.path(),
        &["game.example"],
        |c| c.public_url = Some("https://play.example".into()),
        now(),
    );
    assert_eq!(
        rt.status().https_origins,
        vec!["https://play.example".to_string()]
    );
}

#[test]
fn a_failed_persist_during_regeneration_is_recorded_as_the_last_error() {
    let base = tempfile::tempdir().unwrap();
    let dir = base.path().join("tls");
    let t = now();
    let rt = TlsRuntime::start_with(
        &Config {
            tls_dir: Some(dir.to_string_lossy().into_owned()),
            ..memory_config()
        },
        Arc::new(RcgenGenerator),
        SanSources::default(),
        t,
    );
    assert!(rt.admin_status().last_error.is_none());
    let before = rt.admin_status().fallback_fingerprint.unwrap();
    // The directory becomes unusable: a regular file now occupies its path.
    std::fs::rename(&dir, base.path().join("tls-moved")).unwrap();
    std::fs::write(&dir, b"not a directory").unwrap();
    rt.maintenance_tick(t + Duration::days(370));
    let admin = rt.admin_status();
    assert_ne!(
        admin.fallback_fingerprint.unwrap(),
        before,
        "the regenerated certificate is still served from memory"
    );
    assert!(
        admin.last_error.is_some(),
        "the persist failure must be visible to the admin"
    );
}

#[test]
fn start_logs_the_source_and_fallback_fingerprints_at_info() {
    let dir = tempfile::tempdir().unwrap();
    let t = now();
    let mut runtime = None;
    let log = shadowcat_test_support::logs::capture_info(|| {
        runtime = Some(operator_runtime(dir.path(), &["game.example"], |_| {}, t));
    });
    let (rt, g) = runtime.unwrap();
    let admin = rt.admin_status();
    assert!(log.contains("INFO"), "{log}");
    assert!(
        log.contains(g.loaded.fingerprint()),
        "the operator certificate's fingerprint is logged: {log}"
    );
    assert!(
        log.contains(admin.fallback_fingerprint.as_deref().unwrap()),
        "the fallback fingerprint is logged: {log}"
    );
    assert!(
        log.contains("game.example"),
        "the operator certificate's SAN coverage is logged: {log}"
    );
}

#[test]
fn an_operator_certificate_reload_logs_its_coverage_and_the_installed_fingerprint() {
    let dir = tempfile::tempdir().unwrap();
    let t = now();
    let first = generate(&sans(&["game.example"]), t).unwrap();
    let paths = write_pem(dir.path(), &first);
    let rt = TlsRuntime::start_with(
        &Config {
            tls_cert: Some(paths.cert.to_string_lossy().into_owned()),
            tls_key: Some(paths.key.to_string_lossy().into_owned()),
            ..memory_config()
        },
        Arc::new(RcgenGenerator),
        SanSources::default(),
        t,
    );
    let second = generate(&sans(&["table.example"]), t).unwrap();
    std::fs::write(&paths.cert, &second.cert_pem).unwrap();
    std::fs::write(&paths.key, &second.key_pem).unwrap();
    bump_mtime(&paths.cert, 10);
    bump_mtime(&paths.key, 10);
    let log = shadowcat_test_support::logs::capture_info(|| {
        rt.maintenance_tick(t + Duration::seconds(60))
    });
    assert!(
        log.contains("operator certificate reloaded") && log.contains("table.example"),
        "the reloaded certificate's coverage is logged: {log}"
    );
    assert!(
        log.contains("operator certificate installed") && log.contains(second.loaded.fingerprint()),
        "the installed certificate's fingerprint is logged: {log}"
    );
}

#[test]
fn a_self_signed_regeneration_logs_the_new_fingerprint() {
    let base = tempfile::tempdir().unwrap();
    let t = now();
    let rt = TlsRuntime::start_with(
        &Config {
            tls_dir: Some(base.path().join("tls").to_string_lossy().into_owned()),
            ..memory_config()
        },
        Arc::new(RcgenGenerator),
        SanSources::default(),
        t,
    );
    let before = rt.admin_status().fallback_fingerprint.unwrap();
    let log =
        shadowcat_test_support::logs::capture_info(|| rt.maintenance_tick(t + Duration::days(370)));
    let after = rt.admin_status().fallback_fingerprint.unwrap();
    assert_ne!(before, after);
    assert!(log.contains("regenerated") && log.contains(&after), "{log}");
}
