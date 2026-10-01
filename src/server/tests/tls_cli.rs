//! Process-level tests of the compiled `shadowcat` binary's TLS behaviour: the one-shot
//! `--tls-regenerate` mode and its exclusions, the retired-key boot error, boot errors for
//! contradictory TLS configuration, and a real boot that serves plain HTTP and TLS on one port with
//! a certificate that persists across restarts.

use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use shadowcat_test_support::tls::{handshake_any, sha256_hex};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_shadowcat")
}

/// A `Command` for the binary with every `SHADOWCAT_*` variable removed, so a developer's
/// environment cannot change a result.
fn cmd(dir: &std::path::Path) -> Command {
    let mut c = Command::new(bin());
    c.current_dir(dir);
    for (k, _) in std::env::vars() {
        if k.starts_with("SHADOWCAT_") {
            c.env_remove(k);
        }
    }
    c
}

fn stderr_of(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// How long a one-shot invocation may run before it is killed and the test fails.
const ONE_SHOT_DEADLINE: Duration = Duration::from_secs(60);

/// Runs `c` to completion under [`ONE_SHOT_DEADLINE`]: a binary that hangs (for example one that
/// unexpectedly starts serving) is killed and fails the test instead of stalling the suite.
fn output_within(mut c: Command) -> std::process::Output {
    let mut child = c
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let drain = |mut pipe: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = pipe.read_to_end(&mut buf);
            buf
        })
    };
    let out = drain(Box::new(child.stdout.take().unwrap()));
    let err = drain(Box::new(child.stderr.take().unwrap()));
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if started.elapsed() > ONE_SHOT_DEADLINE {
            let _ = child.kill();
            let _ = child.wait();
            panic!("the binary did not exit within {ONE_SHOT_DEADLINE:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    std::process::Output {
        status,
        stdout: out.join().unwrap(),
        stderr: err.join().unwrap(),
    }
}

#[test]
fn tls_regenerate_is_mutually_exclusive_with_backup_restore_and_subcommands() {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        vec!["--tls-regenerate", "--backup-to", "b"],
        vec!["--tls-regenerate", "--restore-from", "b"],
        vec!["--tls-regenerate", "audio-monitor"],
    ] {
        let mut c = cmd(dir.path());
        c.args(&args);
        let out = output_within(c);
        assert!(!out.status.success(), "{args:?} must fail");
        assert!(
            stderr_of(&out).contains("mutually exclusive"),
            "{args:?}: {}",
            stderr_of(&out)
        );
    }
    // It bails BEFORE Config::load, so it never touched the filesystem.
    assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
}

#[test]
fn tls_regenerate_replaces_the_persisted_certificate_and_prints_its_fingerprint() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("shadowcat.db");
    let run = || {
        let mut c = cmd(dir.path());
        c.args(["--db", db.to_str().unwrap(), "--tls-regenerate"]);
        let out = output_within(c);
        assert!(out.status.success(), "{}", stderr_of(&out));
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let first = run();
    let cert = dir.path().join("tls").join("selfsigned-cert.pem");
    assert!(cert.is_file(), "certificate written beside the db");
    assert!(
        first.contains("regenerated") && first.contains("sha256"),
        "{first}"
    );
    let second = run();
    assert_ne!(
        first, second,
        "a regeneration always produces a new certificate"
    );
}

#[test]
fn tls_regenerate_needs_a_persistent_location() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = cmd(dir.path());
    c.args(["--db", "sqlite::memory:", "--tls-regenerate"]);
    let out = output_within(c);
    assert!(!out.status.success());
    assert!(
        stderr_of(&out).contains("persistent"),
        "{}",
        stderr_of(&out)
    );
}

#[test]
fn the_retired_key_blocks_boot_and_one_shot_modes_and_the_old_flag_is_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = cmd(dir.path());
    c.env("SHADOWCAT_BEHIND_TLS_PROXY", "false")
        .args(["--db", "sqlite::memory:"]);
    let out = output_within(c);
    assert!(!out.status.success());
    assert!(
        stderr_of(&out).contains("behind_proxy"),
        "{}",
        stderr_of(&out)
    );
    // Intentionally also blocks the one-shot modes (they pass through Config::load).
    let mut c = cmd(dir.path());
    c.env("SHADOWCAT_BEHIND_TLS_PROXY", "false")
        .args(["--backup-to", "b"]);
    let out = output_within(c);
    assert!(!out.status.success());
    // The old CLI flag is rejected by clap.
    let mut c = cmd(dir.path());
    c.arg("--behind-tls-proxy");
    let out = output_within(c);
    assert!(!out.status.success());
    assert!(
        stderr_of(&out).contains("--behind-tls-proxy"),
        "{}",
        stderr_of(&out)
    );
}

#[test]
fn contradictory_or_malformed_tls_configuration_is_a_boot_error_naming_the_offending_key() {
    let dir = tempfile::tempdir().unwrap();
    for (args, key) in [
        (
            vec!["--tls", "off", "--tls-cert", "c.pem", "--tls-key", "k.pem"],
            "tls_cert/tls_key",
        ),
        (vec!["--tls-cert", "c.pem"], "tls_key"),
        (
            vec!["--public-url", "http://not-https.example"],
            "public_url",
        ),
        (
            vec!["--public-url", "https://user:pw@example.com"],
            "public_url",
        ),
        (
            vec!["--public-url", "https://example.com/path"],
            "public_url",
        ),
        (
            vec!["--tls-hostnames", "https://scheme.example"],
            "tls_hostnames",
        ),
        (
            vec!["--acme-directory", "http://plain.example/dir"],
            "acme_directory",
        ),
        (vec!["--acme-ca-root", "root.pem"], "acme_ca_root"),
        (vec!["--acme-domains", "game.example"], "acme_terms_agreed"),
    ] {
        let mut c = cmd(dir.path());
        c.args(["--db", "sqlite::memory:", "--bind", "127.0.0.1:0"])
            .args(&args);
        let out = output_within(c);
        assert!(!out.status.success(), "{args:?} must be a boot error");
        assert!(
            stderr_of(&out).contains(key),
            "{args:?}: stderr must name {key}: {}",
            stderr_of(&out)
        );
    }
}

#[test]
fn a_malformed_trusted_proxy_is_a_boot_error_naming_key_and_value() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = cmd(dir.path());
    c.args(["--db", "sqlite::memory:", "--bind", "127.0.0.1:0"])
        .env("SHADOWCAT_TRUSTED_PROXIES", "[not-an-ip]");
    let out = output_within(c);
    assert!(!out.status.success());
    let err = stderr_of(&out);
    assert!(
        err.to_lowercase().contains("trusted_proxies") && err.contains("not-an-ip"),
        "{err}"
    );
}

/// The one-shot modes that touch only the database and assets never consult TLS settings, so a
/// TLS configuration error must not stop a backup or restore.
#[test]
fn backup_and_restore_do_not_validate_tls_settings() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = cmd(dir.path());
    c.args([
        "--db",
        "sqlite::memory:",
        "--tls-cert",
        "c.pem",
        "--backup-to",
        "b",
    ]);
    let out = output_within(c);
    assert!(!out.status.success());
    let err = stderr_of(&out);
    assert!(
        err.contains("in-memory") && !err.contains("tls_key"),
        "the backup fails for its own reason, not the TLS pairing error: {err}"
    );
}

/// `--tls-regenerate` validates TLS settings before acting: a certificate path without its key is
/// rejected naming the missing key.
#[test]
fn tls_regenerate_validates_tls_settings() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("shadowcat.db");
    let mut c = cmd(dir.path());
    c.args([
        "--db",
        db.to_str().unwrap(),
        "--tls-regenerate",
        "--tls-cert",
        "c.pem",
    ]);
    let out = output_within(c);
    assert!(!out.status.success());
    assert!(stderr_of(&out).contains("tls_key"), "{}", stderr_of(&out));
    assert!(
        !dir.path().join("tls").exists(),
        "nothing is generated when validation fails"
    );
}

/// `test_server` validates the TLS settings it is given before it binds or seeds anything.
#[test]
fn test_server_validates_tls_settings() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = Command::new(env!("CARGO_BIN_EXE_test_server"));
    c.current_dir(dir.path()).args(["--tls-cert", "c.pem"]);
    let out = output_within(c);
    assert!(!out.status.success());
    assert!(stderr_of(&out).contains("tls_key"), "{}", stderr_of(&out));
}

#[test]
fn test_server_refuses_a_wildcard_or_malformed_bind() {
    let dir = tempfile::tempdir().unwrap();
    for (bind, needle) in [
        ("0.0.0.0:0", "refused"),
        ("[::]:0", "refused"),
        ("[::ffff:0.0.0.0]:0", "refused"),
        ("localhost:0", "socket address"),
    ] {
        let mut c = Command::new(env!("CARGO_BIN_EXE_test_server"));
        c.current_dir(dir.path()).args(["--bind", bind]);
        let out = output_within(c);
        assert!(!out.status.success(), "{bind} must be refused");
        assert!(
            stderr_of(&out).contains(needle),
            "{bind}: {}",
            stderr_of(&out)
        );
    }
}

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// How long a booting server may take to print its listening address.
const READY_DEADLINE: Duration = Duration::from_secs(60);

/// Boots the server on an OS-assigned port (`--bind 127.0.0.1:0`, so no port is guessed and
/// released) and returns it with the port it actually bound. Readiness is the server's own
/// `reachable at http://127.0.0.1:<port>` startup line, printed after the socket is bound, read
/// under [`READY_DEADLINE`]; a server that exits first, or stays silent, fails the test.
fn boot(dir: &std::path::Path) -> (Server, u16) {
    let mut child = cmd(dir)
        .args([
            "--db",
            dir.join("shadowcat.db").to_str().unwrap(),
            "--bind",
            "127.0.0.1:0",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut server = Server(child);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut sent = false;
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if !sent {
                if let Some(port) = listening_port(&line) {
                    let _ = tx.send(port);
                    sent = true;
                }
            }
            // Keep draining so the server never blocks on a full pipe.
        }
    });
    match rx.recv_timeout(READY_DEADLINE) {
        Ok(port) => (server, port),
        Err(_) => {
            let exited = server.0.try_wait().unwrap();
            panic!("the server printed no listening address (exited: {exited:?})");
        }
    }
}

/// The port in a `reachable at http://127.0.0.1:<port>` startup line, tolerating the ANSI colour
/// codes the log formatter may wrap around it.
fn listening_port(line: &str) -> Option<u16> {
    let rest = line.split("http://127.0.0.1:").nth(1)?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

#[tokio::test]
async fn a_real_boot_serves_http_and_tls_on_one_port_and_the_certificate_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();

    let first_fp = {
        let (_server, port) = boot(dir.path());
        let addr: std::net::SocketAddr = ([127, 0, 0, 1], port).into();
        // Plain HTTP on the shared port.
        let health = reqwest::get(format!("http://127.0.0.1:{port}/health"))
            .await
            .unwrap();
        assert_eq!(health.status(), 200);
        // TLS on the same port, ALPN h2 available, self-signed certificate persisted on disk.
        let (_s, presented) = handshake_any(addr, None, &["h2", "http/1.1"])
            .await
            .unwrap();
        assert_eq!(presented.alpn.as_deref(), Some(&b"h2"[..]));
        assert!(dir.path().join("tls").join("selfsigned-cert.pem").is_file());
        sha256_hex(&presented.leaf)
    };
    // Server dropped (killed). A second boot on the same db reuses the certificate.
    let (_server, port) = boot(dir.path());
    let addr: std::net::SocketAddr = ([127, 0, 0, 1], port).into();
    let (_s, presented) = handshake_any(addr, None, &["http/1.1"]).await.unwrap();
    assert_eq!(sha256_hex(&presented.leaf), first_fp);
}
