//! CLI-level integration tests for `shadowcat audio-monitor`, spawning the actual compiled
//! binary (mirrors `backup_cli.rs`'s `CARGO_BIN_EXE_shadowcat` pattern).

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

fn shadowcat_bin() -> &'static str {
    env!("CARGO_BIN_EXE_shadowcat")
}

#[test]
fn root_flags_still_work_without_a_subcommand() {
    // `--help` on the root Cli exits 0 whether or not a subcommand is present; this is the
    // cheapest proof the flat flags still parse after adding `command: Option<CliCommand>`.
    let status = Command::new(shadowcat_bin())
        .arg("--help")
        .status()
        .expect("run shadowcat --help");
    assert!(status.success());
}

#[test]
fn audio_monitor_dash_dash_port_0_binds_an_ephemeral_port_and_prints_it() {
    let mut child = Command::new(shadowcat_bin())
        .arg("audio-monitor")
        .arg("--port")
        .arg("0")
        .stdout(Stdio::piped())
        .spawn()
        .expect("run shadowcat audio-monitor --port 0");
    let stdout = child.stdout.take().expect("piped stdout");
    let mut reader = BufReader::new(stdout);
    // The tracing diagnostic line ("shadowcat audio-monitor listening") writes to stdout
    // BEFORE the stable, plain-text CLI contract line below it — scan forward past it
    // rather than assuming the announcement is the first line on the stream.
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line).expect("read a stdout line");
        assert_ne!(n, 0, "stream ended before the listening-on line appeared");
        if line.contains("shadowcat audio-monitor listening on 127.0.0.1:") {
            break;
        }
    }
    let port: u16 = line
        .trim()
        .rsplit(':')
        .next()
        .unwrap()
        .parse()
        .expect("a numeric port");
    assert_ne!(
        port, 0,
        "an ephemeral bind must print the ACTUAL bound port, never 0"
    );
    child.kill().expect("stop the audio-monitor process");
    child.wait().expect("reap the audio-monitor process");
}
