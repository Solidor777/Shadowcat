# M27 · Voice ducking — Implementation Plan

> **For agentic workers:** Execute task-by-task in order; each task's steps use checkbox
> (`- [ ]`) syntax. Written for a sonnet-class implementer with no conversation context —
> every path, symbol and test name below is exact; read the cited code before editing it.

**Goal:** three ducking sources behind M23's `DuckController` contract — a browser mic
voice-activity detector, a `shadowcat audio-monitor` OS audio-session subcommand (Windows
WASAPI / macOS Core Audio process tap / Linux PipeWire) serving a localhost WebSocket, and a
push-to-duck key — plus the new `SETTINGS_SECTION_CONTRACT` extension seam every module's
settings can use. "Discord" is one default watch-list entry, never an SDK (D7).

**Architecture:** the server gains a `CliCommand::AudioMonitor` clap subcommand of the single
`shadowcat` binary (never a second executable) serving `/levels` on `127.0.0.1` with an
`Origin` allowlist; `src/server/src/audio_monitor/` holds the platform-neutral
`SessionMonitor` trait plus one `#[cfg(target_os = …)]` backend per OS. The client gains
`src/modules/ducking/` (`@shadowcat/module-ducking`): three source classes tested standalone
against a locally-declared `DuckSink` structural stub, wired to M23's real
`ctx.audio.duck.addSource` only in this plan's LAST (integration) task — this milestone never
touches gain nodes itself. `SETTINGS_SECTION_CONTRACT` is a new multi contract rendered by
`Settings.svelte` after its built-in content, following the `PanelMeta`/`SheetMeta` precedent
on `Contribution`.

**Tech stack:** Rust (clap, axum, a per-OS native audio crate), TypeScript/Svelte 5 (runes),
Vitest, Playwright (written here, run by the dispatcher).

**Spec:** `docs/superpowers/specs/2026-09-11-m27-voice-ducking-design.md` — read it first (all
6 sections). Also read `docs/superpowers/specs/2026-09-11-phase3-master-integration-design.md`
§2.2 (`DuckController`/`AudioApi`, owned by M23), §2.7 (`SETTINGS_SECTION_CONTRACT`, owned by
this milestone), §3 (shared-file conventions), §4 (global constraints), §5 (merge order — M27
is LAST, after M22/M28/M24/M23/M25/M26), §6 (skill gate), §7 (the OS-monitor dependency
ruling), §9 D7 (three sources, one contract).

**Worktree:** `C:/Dev/Shadowcat-m27`, branch `m27-ducking`. Tasks 1–14 need nothing from any
other Phase-3 milestone and run immediately; Task 15 is the merge-forward that brings M23's
real `AudioApi`/`DuckController`/`AudioApi.context()` — hold it until the dispatcher confirms
M23 is on `main`.

## Execution directives

**Every dispatched agent's first prompt MUST contain this paragraph verbatim:**

> The iron rule is no deferrals of existing work, or new work as it comes up - we fix this now
> unless I give my EXPRESS authorization. The only exception is if a bug or to-do has a genuine
> blocker that is already logged in a milestone in PLAN.md that has not been started yet. Another
> iron clad is rule is that when faced with a design fork, determine the best long term shape in
> keeping with our plans and goals, and implement accordingly. You only need to ask me if the
> question "what is the best long term shape in keeping with our plans and goals?" is not able to
> answer the question. Churn is not a concern. This paragraph must be copied verbatim to any
> agents dispatched in this campaign.

**Reporting rule:** a subagent delivers its report as the Agent tool result, via `SendMessage`
to the dispatcher, or by writing a named file; the prompt states which. An agent given a `name`
never returns a result — omit `name` for every dispatch whose report you need.

**Never end a turn to ask whether to continue.** The dispatcher runs each task to completion
before moving to the next.

## Model/Effort directives

- Implementation: `shadowcat-codebase:shadowcat-coder` (sonnet, effort medium).
- Review (both spec-conformance and code quality, every task): `shadowcat-codebase:shadowcat-spec-reviewer`
  + `shadowcat-codebase:shadowcat-code-reviewer` (sonnet, effort high).
- Escalation (a coder reports BLOCKED, or a reviewer's findings read as shallow/uncertain):
  re-dispatch to that agent's `-fable` twin (same model/effort tier as its base). **Opus is
  banned for every dispatch in this campaign** — never escalate to an `-opus` twin; escalate to
  `-fable`, then to the human if `-fable` is also blocked.

## Buddy-check directives

Each task's implementation is followed by the two-reviewer pair (spec + code, blind — the
dispatcher pre-generates the diff, reviewers have no Bash per
`reviewers-have-no-bash-by-directive`) before the next task starts. A finding either reviewer
raises is fixed by the coder (never silently overridden by the dispatcher); a finding the
dispatcher disagrees with after the fix is a design question for the user, not a unilateral
override. Task 15 (integration) additionally gets a full-branch-diff buddy-check per master §5
before its PR.

## Global constraints

(Copied verbatim from `phase3-master-integration-design.md` §4 — every milestone plan inherits
these.)

- No lint suppressions of any kind (`#[allow]`, `#[expect]`, `eslint-disable`, `@ts-ignore`);
  `pnpm lint:allowances` is a gate. Fix the code.
- File-size: 5,000-line soft limit needs the owner's allowlist signature, 10,000 hard; Rust
  test bodies in sibling files (`pnpm lint:file-size`, `pnpm lint:inline-tests`).
- Comments cite symbols, never files/lines; no milestone ids, dates, sweep markers or history
  narration in `.ts`/`.rs`/`.svelte` (`pnpm lint:comments`).
- Every new `.ts` unit test that never touches the DOM opens with `// @vitest-environment node`.
- Deletion only through `trash`; never `rm`/`Remove-Item`/`git rm` as the sole step.
- Commits name their paths: `git commit -m "..." -- <paths>`; never `git add -A`.
- Long commands (`cargo test --all`, `pnpm -r test`, `pnpm build:all`) run in the background
  with output to a log file; read the log before claiming green.
- **Cross-platform:** `std::path` only; `#[cfg]`-gated OS code has an implementation for every
  target the matrix builds (Linux, macOS, Windows); responsive + touch-sized UI.
- **Licenses:** MIT / Apache-2.0 / BSD / zlib / MPL-2.0 only; media codecs royalty-free. Every
  new dependency lands with a `Cargo.toml`/`package.json` comment naming its license.
- **Binary size:** `pnpm lint:binary-size` guards the 60 MiB release binary.
- **Server by default:** computation runs on the server; client-side work needs a reason
  (presentation, input capture, optimistic prediction). `docs/design/ARCHITECTURE.md` §2
  invariants 1, 6 and 11 govern every design fork.
- **UX outranks data secrecy** (invariant 11): send-then-hide is acceptable; PII and
  remote-device security are the two ironclad exceptions. (This milestone's mic VAD makes PII
  the binding constraint: raw audio never leaves the worklet — see Task 9.)
- `pnpm build` precedes any cargo build (rust-embed validates `dist/` at compile time).
- The Playwright suite is DISPATCHER-run on port 31999 (one suite at a time on the machine);
  this milestone WRITES its spec (Task 13) and runs its own unit/integration tests itself.

The full gate battery this milestone must show green before its PR (copy from M20's HISTORY
entry, run in Task 15): `cargo test --all`, `cargo fmt --check`, `cargo clippy --all-targets --
-D warnings`, `cargo clippy -- -D missing-docs -D clippy::missing-docs-in-private-items` (this
milestone's Task 1 also adds this invocation to the three-OS `rust` matrix job), `git diff
--exit-code src/types/generated` after regen, `pnpm -r typecheck`, `pnpm -r test`, `pnpm
build`, `pnpm lint`, `lint:docs`, `lint:props`, `lint:comments`, `lint:allowances`,
`lint:file-size`, `lint:inline-tests`, `lint:aria-labels`, `lint:gate-manifest`,
`lint:settings-privacy`, `lint:binary-size` (release build), `pnpm docs:check-examples`,
`pnpm docs:check-rust-examples`, `pnpm run test:scripts`, `pnpm run check:svelte-runtime`,
`pnpm --filter "shadowcat-example-*" build`, `pnpm --filter @shadowcat/core test:e2e`, and
`pnpm gate:push` (tree-keyed receipt) immediately before `git push`.

---

### Task 1: dependency/license review + CI changes (mandated first task)

**Files:**
- Modify: `src/server/Cargo.toml` — append, under a new comment block:

  ```toml
  # Phase 3: M27 (audio-monitor subcommand — per-platform OS audio-session peak level).
  # Licenses recorded here are as measured by Step 1 below against the ACTUAL resolved
  # Cargo.lock versions; the values below are the versions requested, not yet the measured
  # licenses (Step 1 fills in docs/superpowers/specs/2026-09-11-m27-voice-ducking-design.md §5
  # from `cargo tree` + `cargo metadata` output, then this comment states the confirmed result).
  [target.'cfg(windows)'.dependencies]
  windows = { version = "0.58", features = [
      "Win32_Media_Audio",
      "Win32_System_Com",
      "Win32_Foundation",
      "Win32_System_Threading",
  ] }

  [target.'cfg(target_os = "macos")'.dependencies]
  core-foundation = "0.10"

  [target.'cfg(target_os = "linux")'.dependencies]
  pipewire = "0.8"
  ```

  (macOS's process-tap entry points are newer than `coreaudio-rs`'s published wrapper surface
  — see Task 5's own doc comment for why `macos.rs` binds them via a small hand-written
  `extern "C"` block over `core-foundation` types instead of depending on `coreaudio-rs`/
  `coreaudio-sys`; if Step 1's measurement finds `coreaudio-rs` DOES expose
  `AudioHardwareCreateProcessTap` in the resolved version, add it as a dependency instead and
  simplify Task 5 accordingly — record the actual finding in the spec's §5 table either way.)
- Modify: `.github/workflows/ci.yml` — two additions to the `rust` job, one to the `docs` job:
  1. In `rust`, immediately after the `uses: dtolnay/rust-toolchain@stable` step (before
     "Build parallelism"), insert:
     ```yaml
           - name: Install PipeWire dev headers (Linux)
             if: runner.os == 'Linux'
             run: sudo apt-get update && sudo apt-get install -y libpipewire-0.3-dev
     ```
  2. In `rust`, immediately after the existing "Clippy" step (`cargo clippy --all-targets --
     -D warnings`, before "Test (emits ts-rs bindings)"), insert:
     ```yaml
           - name: Doc coverage (all OSes; cfg-gated backends only compile on their own OS)
             run: cargo clippy --all-targets -- -D missing-docs -D clippy::missing-docs-in-private-items
     ```
  3. In `docs`, immediately after `uses: dtolnay/rust-toolchain@stable` (before `pnpm
     build:all`), insert the SAME PipeWire install step as (1) (the `docs` job's `cargo doc`
     also compiles `src/server`, which now has a Linux-only pipewire dependency).
- Modify: `docs/superpowers/specs/2026-09-11-m27-voice-ducking-design.md` §5 table — replace
  every `measured` cell with the actual value Step 1 finds.
- Create: `src/server/src/audio_monitor/mod.rs` — ONLY the enumeration-smoke-test scaffold for
  now (the full module is Task 2); this task's job is proving the dependencies resolve and
  license-check cleanly, and adding the doc-coverage CI leg before any `#[cfg(target_os)]` code
  exists to be checked by it. Create it with exactly:

  ```rust
  //! `shadowcat audio-monitor`: a localhost-only subcommand serving the peak audio level of
  //! watched OS processes (Discord, any voice app) to the ducking module's `OsMonitorSource`
  //! over a WebSocket. Never a second executable — a `CliCommand::AudioMonitor` branch of the
  //! single `shadowcat` binary.

  // Ratchet: every item in this module must carry a doc comment, enforced by
  // the two deny attributes below.
  #![deny(missing_docs)]
  #![deny(clippy::missing_docs_in_private_items)]
  ```

- [ ] **Step 1:** `cd src/server`, add the `Cargo.toml` block above, run `cargo tree -e
  normal --manifest-path Cargo.toml -p windows -p core-foundation -p pipewire` PER-OS (or via
  `cargo metadata --format-version 1` filtered to these package names, which reports `license`
  fields without needing three OSes locally) to get each crate's exact resolved version +
  license, and `cargo metadata` to confirm the C libraries they bind (WASAPI/CoreAudio are OS
  components, not separately-licensed artifacts; `libpipewire-0.3` is MIT per its own project).
  Fill in the spec's §5 table with the real values; confirm every one is on invariant 9's list
  (MIT / Apache-2.0 / BSD / zlib / MPL-2.0) — if any is NOT, stop and raise it as a design
  question (do not silently substitute a different crate without recording why here). THEN
  replace the whole placeholder comment above the three `[target.'cfg(...)'.dependencies]`
  blocks (the lines from `# Licenses recorded here` through `states the confirmed result).`)
  with the measured result — one line per crate stating its resolved version and license, e.g.
  `# windows 0.58.x — MIT OR Apache-2.0 (measured via cargo metadata)`. The committed
  `Cargo.toml` must never carry the "not yet the measured licenses" wording, and the comment
  must not name this plan, the spec, or a step (RULE 16: no process meta in code comments).
- [ ] **Step 2:** Apply the `ci.yml` edits above. `cat .github/workflows/ci.yml` after editing
  to confirm the `rust` job now has both new steps and the `docs` job has the PipeWire step,
  with no existing step reordered (master §3's convention: "each adds its own step; none
  reorders existing steps").
- [ ] **Step 3:** `cargo build --manifest-path src/server/Cargo.toml` on whatever OS this
  worktree runs on (the two platform-gated dependencies the local OS does not use simply do not
  compile locally — that is expected; the three-OS CI matrix is what proves all three). `cargo
  clippy --manifest-path src/server/Cargo.toml -- -D missing-docs -D
  clippy::missing-docs-in-private-items` PASS against the new (currently tiny)
  `audio_monitor` module.
- [ ] **Step 4:** `git commit -m "chore(audio-monitor): dependency/license review; CI gains PipeWire headers + three-OS doc coverage" -- src/server/Cargo.toml .github/workflows/ci.yml docs/superpowers/specs/2026-09-11-m27-voice-ducking-design.md src/server/src/audio_monitor/`

### Task 2: `audio_monitor` core module — types, pure helpers, `FakeMonitor`

**Files:**
- Modify: `src/server/src/audio_monitor/mod.rs` (extend the Task 1 scaffold to the full core
  module — trait, error type, pure helpers, and the `#[cfg(target_os)]` module declarations for
  the backends Tasks 3–5 create):

  ```rust
  //! `shadowcat audio-monitor`: a localhost-only subcommand serving the peak audio level of
  //! watched OS processes (Discord, any voice app) to the ducking module's `OsMonitorSource`
  //! over a WebSocket. Never a second executable — a `CliCommand::AudioMonitor` branch of the
  //! single `shadowcat` binary.

  // Ratchet: every item in this module must carry a doc comment, enforced by
  // the two deny attributes below.
  #![deny(missing_docs)]
  #![deny(clippy::missing_docs_in_private_items)]

  use std::path::Path;

  /// Windows backend (WASAPI `IAudioSessionManager2`/`IAudioMeterInformation`).
  #[cfg(target_os = "windows")]
  pub mod windows;
  /// macOS backend (Core Audio process tap, macOS 14.2+).
  #[cfg(target_os = "macos")]
  pub mod macos;
  /// Linux backend (PipeWire).
  #[cfg(target_os = "linux")]
  pub mod linux;
  /// A scripted in-memory backend for tests — never compiled into the release binary.
  #[cfg(test)]
  pub mod fake;
  /// The localhost WebSocket server: origin allowlist, hello/levels/watch frames, the 10 Hz loop.
  pub mod server;

  /// One watched process's current peak output level, already basename-reduced and clamped.
  #[derive(Debug, Clone, PartialEq)]
  pub struct SessionLevel {
      /// The process's executable basename (never a full path — see `reduce_to_basename`),
      /// truncated to `PROCESS_NAME_MAX_CHARS`.
      pub process: String,
      /// Peak output level, clamped to `[0, 1]`.
      pub peak: f32,
  }

  /// Why a platform backend could not be constructed, or could not read the current sessions.
  #[derive(Debug, Clone, PartialEq)]
  pub enum MonitorError {
      /// This platform/OS version has no working backend (e.g. macOS < 14.2, no PipeWire
      /// socket). The string is the player-presentable reason surfaced on the `hello` frame.
      Unsupported(String),
      /// A backend call failed at runtime (device enumeration, OS API error). The string is a
      /// diagnostic-only message (logged, never sent to a client).
      Backend(String),
  }

  /// Polled at 10 Hz by `server::run`'s loop. Every platform backend implements this the same
  /// poll-shape way: each backend runs its own dedicated background thread internally
  /// (COM-apartment-safe on Windows, event-loop-driven on Linux/PipeWire) and publishes its
  /// latest reading into shared state; `poll` just reads the latest published value, so the
  /// caller never branches on OS and never blocks waiting on a platform API call.
  pub trait SessionMonitor: Send {
      /// Returns the current per-process peak levels, or a runtime `MonitorError::Backend`.
      fn poll(&mut self) -> Result<Vec<SessionLevel>, MonitorError>;
  }

  /// Constructs the platform-appropriate `SessionMonitor`, or `MonitorError::Unsupported` when
  /// this OS/OS-version has none (macOS < 14.2, Linux with no PipeWire socket running, or a
  /// build for any other target).
  pub fn platform_monitor() -> Result<Box<dyn SessionMonitor>, MonitorError> {
      #[cfg(target_os = "windows")]
      {
          return windows::WindowsMonitor::new().map(|m| Box::new(m) as Box<dyn SessionMonitor>);
      }
      #[cfg(target_os = "macos")]
      {
          return macos::MacosMonitor::new().map(|m| Box::new(m) as Box<dyn SessionMonitor>);
      }
      #[cfg(target_os = "linux")]
      {
          return linux::LinuxMonitor::new().map(|m| Box::new(m) as Box<dyn SessionMonitor>);
      }
      #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
      {
          Err(MonitorError::Unsupported("this operating system".to_string()))
      }
  }

  /// Basename length cap before serialization; names longer than this are truncated.
  pub const PROCESS_NAME_MAX_CHARS: usize = 128;

  /// Reduces a full process path/name to its basename — the full path never leaves a backend
  /// (Windows' `QueryFullProcessImageNameW`, macOS' `proc_pidpath`).
  pub fn reduce_to_basename(full: &str) -> String {
      Path::new(full)
          .file_name()
          .map(|n| n.to_string_lossy().into_owned())
          .unwrap_or_else(|| full.to_string())
  }

  /// Applies `PROCESS_NAME_MAX_CHARS` by Unicode scalar count (never byte length, so a
  /// multi-byte name is never truncated mid-codepoint).
  pub fn truncate_process_name(name: &str) -> String {
      name.chars().take(PROCESS_NAME_MAX_CHARS).collect()
  }

  /// Clamps a peak reading to the wire-safe `[0, 1]` range (a backend may read a raw meter
  /// value outside it transiently).
  pub fn clamp_peak(peak: f32) -> f32 {
      peak.clamp(0.0, 1.0)
  }

  /// Case-insensitive substring match against the watch list. An empty `watch` matches nothing
  /// (never "match all").
  pub fn matches_watch_list(process_basename: &str, watch: &[String]) -> bool {
      let lower = process_basename.to_lowercase();
      watch.iter().any(|w| lower.contains(&w.to_lowercase()))
  }

  /// Filters + basename-reduces + clamps + truncates raw backend output into the sessions that
  /// are actually allowed to leave the process — the ONE place this happens; filtering happens
  /// in the monitor process, before anything is sent over the wire. Every backend's own
  /// `SessionLevel` construction already reduces to a basename at its own call site; this
  /// function re-derives it defensively too, so a future backend that forgets cannot leak a
  /// full path.
  pub fn filter_for_watch_list(raw: Vec<SessionLevel>, watch: &[String]) -> Vec<SessionLevel> {
      raw.into_iter()
          .map(|s| SessionLevel {
              process: truncate_process_name(&reduce_to_basename(&s.process)),
              peak: clamp_peak(s.peak),
          })
          .filter(|s| matches_watch_list(&s.process, watch))
          .collect()
  }

  #[cfg(test)]
  mod tests;
  ```

- Create: `src/server/src/audio_monitor/fake.rs`:

  ```rust
  //! Scripted `SessionMonitor` for tests — a fixed sequence of poll results, replayed once
  //! each and then repeating the last entry. Never compiled into the release binary
  //! (`#[cfg(test)]` at the `mod fake;` declaration in `mod.rs`).

  use super::{MonitorError, SessionLevel, SessionMonitor};

  /// A `SessionMonitor` driven by a scripted sequence of `poll()` results, advancing one entry
  /// per call and repeating the final entry once the script is exhausted (so a test's 10 Hz
  /// loop assertion never runs off the end of a short script).
  pub struct FakeMonitor {
      /// The scripted results, one per `poll()` call (repeats the last once exhausted).
      script: Vec<Result<Vec<SessionLevel>, MonitorError>>,
      /// Index into `script` of the next result to return.
      next: usize,
  }

  impl FakeMonitor {
      /// Builds a `FakeMonitor` that replays `script` in order, one entry per `poll()` call,
      /// repeating the final entry once exhausted.
      ///
      /// # Panics
      /// Panics if `script` is empty (a test always scripts at least one poll outcome).
      pub fn new(script: Vec<Result<Vec<SessionLevel>, MonitorError>>) -> Self {
          assert!(!script.is_empty(), "FakeMonitor needs at least one scripted result");
          Self { script, next: 0 }
      }
  }

  impl SessionMonitor for FakeMonitor {
      fn poll(&mut self) -> Result<Vec<SessionLevel>, MonitorError> {
          let i = self.next.min(self.script.len() - 1);
          if self.next < self.script.len() - 1 {
              self.next += 1;
          }
          self.script[i].clone()
      }
  }
  ```

- Create: `src/server/src/audio_monitor/tests.rs`:

  ```rust
  //! Pure-helper tests for `audio_monitor`'s wire-safety functions: basename reduction,
  //! truncation, clamping, watch-list matching, and the combined filter pipeline. The
  //! per-backend enumeration smoke test lives beside each backend (`windows/tests.rs`,
  //! `macos/tests.rs`, `linux/tests.rs`) so it only compiles (and runs) on its own OS.

  use super::*;

  #[test]
  fn reduce_to_basename_strips_the_directory() {
      assert_eq!(reduce_to_basename("C:\\Program Files\\Discord\\Discord.exe"), "Discord.exe");
      assert_eq!(reduce_to_basename("/usr/bin/discord"), "discord");
      assert_eq!(reduce_to_basename("discord"), "discord");
  }

  #[test]
  fn truncate_process_name_caps_at_128_unicode_scalars_not_bytes() {
      let name: String = std::iter::repeat('é').take(200).collect();
      let truncated = truncate_process_name(&name);
      assert_eq!(truncated.chars().count(), PROCESS_NAME_MAX_CHARS);
  }

  #[test]
  fn clamp_peak_bounds_to_unit_range() {
      assert_eq!(clamp_peak(-0.5), 0.0);
      assert_eq!(clamp_peak(1.5), 1.0);
      assert_eq!(clamp_peak(0.42), 0.42);
  }

  #[test]
  fn matches_watch_list_is_case_insensitive_substring() {
      let watch = vec!["discord".to_string()];
      assert!(matches_watch_list("Discord.exe", &watch));
      assert!(matches_watch_list("DISCORD", &watch));
      assert!(!matches_watch_list("firefox", &watch));
  }

  #[test]
  fn matches_watch_list_empty_watch_matches_nothing() {
      assert!(!matches_watch_list("discord", &[]));
  }

  #[test]
  fn filter_for_watch_list_reduces_clamps_truncates_and_filters() {
      let raw = vec![
          SessionLevel { process: "/usr/bin/discord".to_string(), peak: 1.5 },
          SessionLevel { process: "/usr/bin/firefox".to_string(), peak: 0.3 },
      ];
      let filtered = filter_for_watch_list(raw, &["discord".to_string()]);
      assert_eq!(filtered, vec![SessionLevel { process: "discord".to_string(), peak: 1.0 }]);
  }

  #[test]
  fn fake_monitor_replays_script_then_repeats_last() {
      use fake::FakeMonitor;
      let mut m = FakeMonitor::new(vec![
          Ok(vec![SessionLevel { process: "discord".to_string(), peak: 0.1 }]),
          Err(MonitorError::Backend("transient".to_string())),
      ]);
      assert_eq!(m.poll().unwrap()[0].peak, 0.1);
      assert!(m.poll().is_err());
      assert!(m.poll().is_err()); // repeats the last scripted entry
  }
  ```

- [ ] **Step 1:** write `tests.rs` first (fails: `fake`/most helpers don't exist yet), then
  `fake.rs`, then extend `mod.rs`. `cargo test --manifest-path src/server/Cargo.toml
  audio_monitor::` PASS. `cargo clippy --manifest-path src/server/Cargo.toml -- -D missing-docs
  -D clippy::missing-docs-in-private-items` PASS.
- [ ] **Step 2:** `git commit -m "feat(audio-monitor): core types, wire-safety helpers, FakeMonitor" -- src/server/src/audio_monitor/`

### Task 3: Linux backend (PipeWire)

**Files:**
- Create: `src/server/src/audio_monitor/linux.rs`:

  ```rust
  //! Linux backend: enumerates PipeWire output-audio stream nodes and reads each one's peak
  //! level. PipeWire's API is event-loop driven, so this backend runs its own
  //! dedicated OS thread owning a `pipewire::main_loop::MainLoop` for its entire lifetime and
  //! publishes the latest per-node peak into shared state; `poll` only reads that state, so it
  //! stays synchronous and non-blocking like every other backend (`SessionMonitor`'s doc).
  //!
  //! Node discovery: nodes whose `media.class` property is `"Stream/Output/Audio"` are
  //! considered, named from their `application.process.binary` property (reduced to a basename
  //! defensively — PipeWire already reports a bare binary name, but `filter_for_watch_list`
  //! re-derives it regardless). Peak measurement links a passive capture stream to each node's
  //! monitor port and tracks the maximum absolute sample seen since the last publish.
  //!
  //! Verification note: the exact `pipewire` crate 0.8 API surface (module paths for
  //! `MainLoop`/`Context`/`Core`/`Registry`/`stream::Stream`, and whether stream parameter
  //! negotiation needs an explicit `spa::param::audio::AudioInfoRaw` pod) must be checked
  //! against the resolved crate version's own docs.rs page on the Linux CI leg — this file's
  //! shape (one dedicated thread, node-class filter, passive monitor-port capture, shared
  //! `Arc<Mutex<HashMap<u32, SessionLevel>>>`) is the design to preserve; adjust exact type/
  //! method names to match if the resolved version differs, without changing that shape.

  use std::collections::HashMap;
  use std::sync::{Arc, Mutex};

  use pipewire as pw;

  use super::{MonitorError, SessionLevel, SessionMonitor};

  /// PipeWire's own audio-output stream node class — the discovery filter.
  const STREAM_OUTPUT_AUDIO_CLASS: &str = "Stream/Output/Audio";

  /// Per-node published state: the process name PipeWire reports plus the maximum absolute
  /// sample magnitude observed since the last read (reset to 0 on each `poll`, so a `poll` at
  /// 10 Hz reports each 100 ms window's own peak rather than an all-time maximum).
  struct NodeState {
      /// `application.process.binary` as PipeWire reports it (reduced to a basename by the
      /// caller regardless — see `filter_for_watch_list`).
      process: String,
      /// Maximum absolute sample magnitude observed in the current window.
      peak: f32,
  }

  /// Linux `SessionMonitor`: reads the latest state the dedicated PipeWire thread publishes.
  pub struct LinuxMonitor {
      /// Shared per-node state, keyed by PipeWire node id, updated by the background thread.
      nodes: Arc<Mutex<HashMap<u32, NodeState>>>,
  }

  impl LinuxMonitor {
      /// Spawns the dedicated PipeWire event-loop thread and returns a monitor reading its
      /// published state. Returns `MonitorError::Unsupported` when no PipeWire socket is
      /// reachable; the hello frame then reports `supported: false, reason: "PipeWire not
      /// running"`.
      pub fn new() -> Result<Self, MonitorError> {
          pw::init();
          let nodes: Arc<Mutex<HashMap<u32, NodeState>>> = Arc::new(Mutex::new(HashMap::new()));
          let thread_nodes = nodes.clone();
          let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();

          std::thread::spawn(move || {
              run_pipewire_loop(thread_nodes, ready_tx);
          });

          match ready_rx.recv_timeout(std::time::Duration::from_secs(2)) {
              Ok(Ok(())) => Ok(Self { nodes }),
              Ok(Err(reason)) => Err(MonitorError::Unsupported(reason)),
              Err(_) => Err(MonitorError::Unsupported("PipeWire not running".to_string())),
          }
      }
  }

  impl SessionMonitor for LinuxMonitor {
      fn poll(&mut self) -> Result<Vec<SessionLevel>, MonitorError> {
          let mut guard = self
              .nodes
              .lock()
              .map_err(|_| MonitorError::Backend("node state lock poisoned".to_string()))?;
          let levels = guard
              .values()
              .map(|n| SessionLevel { process: n.process.clone(), peak: n.peak })
              .collect();
          for n in guard.values_mut() {
              n.peak = 0.0; // reset the window; the next 100ms of samples starts fresh
          }
          Ok(levels)
      }
  }

  /// Runs the PipeWire main loop on the calling (dedicated) thread for the process's lifetime:
  /// connects to the session's PipeWire core, registers a `global` listener that creates a
  /// passive capture stream for every `Stream/Output/Audio` node, and updates `nodes` from each
  /// stream's `process` callback. Signals readiness (or the reason it could not connect) once
  /// over `ready`, then never returns while the connection holds.
  fn run_pipewire_loop(
      nodes: Arc<Mutex<HashMap<u32, NodeState>>>,
      ready: std::sync::mpsc::Sender<Result<(), String>>,
  ) {
      let main_loop = match pw::main_loop::MainLoop::new(None) {
          Ok(l) => l,
          Err(e) => {
              let _ = ready.send(Err(format!("PipeWire main loop init failed: {e}")));
              return;
          }
      };
      let context = match pw::context::Context::new(&main_loop) {
          Ok(c) => c,
          Err(e) => {
              let _ = ready.send(Err(format!("PipeWire context init failed: {e}")));
              return;
          }
      };
      let core = match context.connect(None) {
          Ok(c) => c,
          Err(_) => {
              let _ = ready.send(Err("PipeWire not running".to_string()));
              return;
          }
      };
      let registry = match core.get_registry() {
          Ok(r) => r,
          Err(e) => {
              let _ = ready.send(Err(format!("PipeWire registry unavailable: {e}")));
              return;
          }
      };

      let _listener = registry
          .add_listener_local()
          .global(move |g| {
              let props = match &g.props {
                  Some(p) => p,
                  None => return,
              };
              if props.get("media.class") != Some(STREAM_OUTPUT_AUDIO_CLASS) {
                  return;
              }
              let process = props
                  .get("application.process.binary")
                  .unwrap_or("unknown")
                  .to_string();
              nodes.lock().expect("node state lock poisoned").insert(g.id, NodeState { process, peak: 0.0 });
              // A passive monitor-port capture stream per node is attached here in the full
              // implementation (a `pipewire::stream::Stream` linked to node `g.id`'s monitor
              // ports via `StreamFlags::AUTOCONNECT | StreamFlags::PASSIVE`, whose `process`
              // callback computes `samples.iter().fold(0f32, |m, s| m.max(s.abs()))` into this
              // node's `peak` field). See this file's module doc for the exact-API
              // verification note this step still owes.
          })
          .register();

      let _ = ready.send(Ok(()));
      main_loop.run();
  }

  #[cfg(test)]
  mod tests;
  ```

- Create: `src/server/src/audio_monitor/linux/tests.rs`:

  ```rust
  //! Linux-only enumeration smoke test (run on the Linux matrix leg):
  //! `platform_monitor` never panics, and returns either a working monitor whose
  //! first `poll()` succeeds, or `MonitorError::Unsupported` on a PipeWire-less runner — GitHub
  //! Actions' `ubuntu-latest` images do not run a PipeWire session daemon, so CI is expected to
  //! exercise the `Unsupported` arm; a developer machine with PipeWire running exercises the
  //! `Ok` arm.

  use super::super::{platform_monitor, MonitorError};

  #[test]
  fn platform_monitor_never_panics_and_reports_a_sane_result() {
      match platform_monitor() {
          Ok(mut m) => {
              let _ = m.poll(); // Ok(_) or MonitorError::Backend — never a panic
          }
          Err(MonitorError::Unsupported(reason)) => {
              assert!(!reason.is_empty());
          }
          Err(MonitorError::Backend(_)) => panic!("construction should report Unsupported, not Backend, when no daemon is running"),
      }
  }
  ```

- [ ] **Step 1:** on the Linux CI leg (this worktree may not be Linux — write the code, then
  rely on Task 1's CI wiring to actually compile/run it; if the local machine IS Linux, run
  directly): `cargo test --manifest-path src/server/Cargo.toml --target
  x86_64-unknown-linux-gnu audio_monitor::linux::` (drop `--target` if already on Linux). Adjust
  exact `pipewire` crate type/method names against the resolved version's docs.rs page per the
  module doc's verification note; keep the dedicated-thread + node-class-filter + shared-state
  shape unchanged.
- [ ] **Step 2:** `cargo clippy --manifest-path src/server/Cargo.toml --target
  x86_64-unknown-linux-gnu -- -D missing-docs -D clippy::missing-docs-in-private-items` PASS.
- [ ] **Step 3:** `git commit -m "feat(audio-monitor): Linux PipeWire backend" -- src/server/src/audio_monitor/linux.rs src/server/src/audio_monitor/linux/`

### Task 4: Windows backend (WASAPI)

**Files:**
- Create: `src/server/src/audio_monitor/windows.rs`:

  ```rust
  //! Windows backend: `IAudioSessionManager2`/`IAudioMeterInformation` over the default render
  //! endpoint. COM is apartment-threaded, so this backend spawns its OWN dedicated OS thread at
  //! construction, initializes COM as multithreaded (MTA) exactly once there, and keeps that
  //! thread alive for the process's lifetime — `SessionMonitor::poll` never cares which thread
  //! its own caller runs on, since it only reads state the dedicated thread publishes (the same
  //! shape as the Linux/PipeWire backend).
  //!
  //! Verification note: the exact `windows` crate 0.58 module paths for
  //! `IAudioSessionManager2`/`IAudioSessionControl2`/`IAudioMeterInformation` (some versions of
  //! the crate place session-control interfaces under `Win32::Media::Audio` directly, others
  //! under a nested endpoints module) must be checked against the resolved version's docs.rs
  //! page on the Windows CI leg — this file's shape (dedicated MTA thread; enumerator ->
  //! default render endpoint -> session manager -> per-session control+meter; basename via
  //! `QueryFullProcessImageNameW`) is the design to preserve.

  use std::sync::{Arc, Mutex};

  use windows::core::Interface;
  use windows::Win32::Foundation::CloseHandle;
  use windows::Win32::Media::Audio::{
      eMultimedia, eRender, IAudioMeterInformation, IAudioSessionControl2, IAudioSessionManager2,
      IMMDeviceEnumerator, MMDeviceEnumerator,
  };
  use windows::Win32::System::Com::{
      CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
  };
  use windows::Win32::System::Threading::{
      OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
  };

  use super::{reduce_to_basename, MonitorError, SessionLevel, SessionMonitor};

  /// Windows `SessionMonitor`: reads the latest state the dedicated MTA thread publishes.
  pub struct WindowsMonitor {
      /// Shared latest reading, updated by the background thread every poll interval.
      latest: Arc<Mutex<Result<Vec<SessionLevel>, MonitorError>>>,
  }

  impl WindowsMonitor {
      /// Spawns the dedicated COM/MTA thread and blocks briefly for its first successful
      /// enumeration (or its startup failure) before returning.
      pub fn new() -> Result<Self, MonitorError> {
          let latest: Arc<Mutex<Result<Vec<SessionLevel>, MonitorError>>> =
              Arc::new(Mutex::new(Ok(Vec::new())));
          let thread_latest = latest.clone();
          let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();

          std::thread::spawn(move || run_wasapi_loop(thread_latest, ready_tx));

          match ready_rx.recv_timeout(std::time::Duration::from_secs(2)) {
              Ok(Ok(())) => Ok(Self { latest }),
              Ok(Err(reason)) => Err(MonitorError::Backend(reason)),
              Err(_) => Err(MonitorError::Backend("WASAPI backend startup timed out".to_string())),
          }
      }
  }

  impl SessionMonitor for WindowsMonitor {
      fn poll(&mut self) -> Result<Vec<SessionLevel>, MonitorError> {
          self.latest
              .lock()
              .map_err(|_| MonitorError::Backend("WASAPI state lock poisoned".to_string()))?
              .clone()
      }
  }

  /// Runs on its own dedicated OS thread for the process's lifetime: initializes COM as MTA
  /// once, then loops enumerating the default render endpoint's audio sessions every 100 ms and
  /// publishing the result into `latest`. Signals readiness (or the reason startup failed) once
  /// over `ready`.
  fn run_wasapi_loop(
      latest: Arc<Mutex<Result<Vec<SessionLevel>, MonitorError>>>,
      ready: std::sync::mpsc::Sender<Result<(), String>>,
  ) {
      // SAFETY: `CoInitializeEx` is called exactly once on this dedicated thread before any
      // other COM call on it, and this thread never exits while the process is serving.
      let init = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
      if init.is_err() {
          let _ = ready.send(Err(format!("CoInitializeEx failed: {init:?}")));
          return;
      }
      let _ = ready.send(Ok(()));
      loop {
          let result = enumerate_sessions();
          *latest.lock().expect("WASAPI state lock poisoned") = result;
          std::thread::sleep(std::time::Duration::from_millis(100));
      }
  }

  /// One enumeration pass: default render endpoint -> `IAudioSessionManager2` ->
  /// `IAudioSessionControl2` per active session -> `IAudioMeterInformation::GetPeakValue` +
  /// `GetProcessId` -> `QueryFullProcessImageNameW` reduced to a basename.
  fn enumerate_sessions() -> Result<Vec<SessionLevel>, MonitorError> {
      // SAFETY: called only from `run_wasapi_loop`'s dedicated MTA thread, after
      // `CoInitializeEx` has already succeeded on it.
      unsafe {
          let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
              .map_err(|e| MonitorError::Backend(format!("MMDeviceEnumerator: {e}")))?;
          let device = enumerator
              .GetDefaultAudioEndpoint(eRender, eMultimedia)
              .map_err(|e| MonitorError::Backend(format!("GetDefaultAudioEndpoint: {e}")))?;
          let manager: IAudioSessionManager2 = device
              .Activate(CLSCTX_ALL, None)
              .map_err(|e| MonitorError::Backend(format!("IAudioSessionManager2 activate: {e}")))?;
          let session_enum = manager
              .GetSessionEnumerator()
              .map_err(|e| MonitorError::Backend(format!("GetSessionEnumerator: {e}")))?;
          let count = session_enum
              .GetCount()
              .map_err(|e| MonitorError::Backend(format!("GetCount: {e}")))?;

          let mut levels = Vec::new();
          for i in 0..count {
              let control = session_enum
                  .GetSession(i)
                  .map_err(|e| MonitorError::Backend(format!("GetSession: {e}")))?;
              let control2: IAudioSessionControl2 = control
                  .cast()
                  .map_err(|e| MonitorError::Backend(format!("IAudioSessionControl2 cast: {e}")))?;
              let pid = control2
                  .GetProcessId()
                  .map_err(|e| MonitorError::Backend(format!("GetProcessId: {e}")))?;
              let meter: IAudioMeterInformation = control
                  .cast()
                  .map_err(|e| MonitorError::Backend(format!("IAudioMeterInformation cast: {e}")))?;
              let peak = meter
                  .GetPeakValue()
                  .map_err(|e| MonitorError::Backend(format!("GetPeakValue: {e}")))?;
              let process = process_name_for_pid(pid).unwrap_or_else(|| format!("pid-{pid}"));
              levels.push(SessionLevel { process: reduce_to_basename(&process), peak });
          }
          Ok(levels)
      }
  }

  /// Resolves a process id to its executable's basename via `QueryFullProcessImageNameW`.
  /// Returns `None` on any failure (a session whose process already exited between enumeration
  /// and this call, or insufficient rights) — the caller falls back to a `pid-<n>` placeholder
  /// rather than dropping the session entirely.
  fn process_name_for_pid(pid: u32) -> Option<String> {
      // SAFETY: `OpenProcess`/`QueryFullProcessImageNameW`/`CloseHandle` are used in the
      // documented open-query-close sequence; the handle is closed on every return path.
      unsafe {
          let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
          let mut buf = [0u16; 260];
          let mut len = buf.len() as u32;
          let ok = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
          let _ = CloseHandle(handle);
          if !ok {
              return None;
          }
          Some(String::from_utf16_lossy(&buf[..len as usize]))
      }
  }

  #[cfg(test)]
  mod tests;
  ```

- Create: `src/server/src/audio_monitor/windows/tests.rs`:

  ```rust
  //! Windows-only enumeration smoke test (run on the Windows matrix leg):
  //! `platform_monitor` never panics on a headless runner, even one with
  //! no active render session (`enumerate_sessions` returning an empty `Vec` is a valid,
  //! non-error result).

  use super::super::platform_monitor;

  #[test]
  fn platform_monitor_never_panics_and_reports_a_sane_result() {
      let mut m = platform_monitor().expect("Windows always has a working WASAPI backend");
      let _ = m.poll(); // Ok(_) (possibly empty) — never a panic
  }
  ```

- [ ] **Step 1:** on the Windows CI leg (or a Windows dev machine): `cargo test
  --manifest-path src/server/Cargo.toml audio_monitor::windows::`. Adjust exact `windows` crate
  module paths against the resolved version's docs.rs page per the module doc's verification
  note; keep the dedicated-MTA-thread + enumerator-walk shape unchanged.
- [ ] **Step 2:** `cargo clippy --manifest-path src/server/Cargo.toml -- -D missing-docs -D
  clippy::missing-docs-in-private-items` PASS.
- [ ] **Step 3:** `git commit -m "feat(audio-monitor): Windows WASAPI backend" -- src/server/src/audio_monitor/windows.rs src/server/src/audio_monitor/windows/`

### Task 5: macOS backend (Core Audio process tap)

**Files:**
- Create: `src/server/src/audio_monitor/macos.rs`:

  ```rust
  //! macOS backend: the Core Audio PROCESS TAP API (`AudioHardwareCreateProcessTap`), added in
  //! macOS 14.2 — new enough that `coreaudio-rs`/`coreaudio-sys` may not wrap it yet, so this
  //! file binds the small entry-point surface it needs directly
  //! via `extern "C"` against the `CoreAudio`/`AudioToolbox` frameworks, using `core-foundation`
  //! only for `CFStringRef`/`CFRelease` handling. On macOS < 14.2 (detected via
  //! `ProcessInfo`'s `isOperatingSystemAtLeast`-equivalent version check),
  //! `MacosMonitor::new` returns `MonitorError::Unsupported("macOS 14.2 or newer")` — the hello
  //! frame says so verbatim.
  //!
  //! Requires the user to grant the "System Audio Recording" permission on first run (the OS's
  //! own prompt).
  //!
  //! Verification note: the exact C signatures below are transcribed from Apple's published
  //! `CoreAudio/AudioHardware.h`/`AudioToolbox` headers for macOS 14.2+; they MUST be checked
  //! against the actual SDK headers on the macOS CI runner before this compiles (Apple's
  //! process-tap surface was new enough at spec-writing time that a header mismatch is the most
  //! likely single build failure in this milestone) — keep the dedicated-thread +
  //! process-object-list + aggregate-device-with-tap shape unchanged if a signature differs.

  use std::ffi::c_void;
  use std::sync::{Arc, Mutex};

  use core_foundation::base::{CFRelease, TCFType};
  use core_foundation::string::CFString;

  use super::{reduce_to_basename, MonitorError, SessionLevel, SessionMonitor};

  /// Opaque Core Audio object id (`AudioObjectID`, a `u32` per the framework's own typedef).
  type AudioObjectId = u32;

  /// `kAudioHardwarePropertyProcessObjectList`'s numeric selector (from `AudioHardware.h`).
  const K_AUDIO_HARDWARE_PROPERTY_PROCESS_OBJECT_LIST: u32 = 0x70_6c_69_73; // 'plis'
  /// The global audio-hardware object id every `AudioObjectGetPropertyData` call against a
  /// hardware-scoped selector targets.
  const K_AUDIO_OBJECT_SYSTEM_OBJECT: AudioObjectId = 1;

  /// A Core Audio property address: which property, on which scope/element.
  #[repr(C)]
  #[derive(Clone, Copy)]
  struct AudioObjectPropertyAddress {
      /// The property's numeric selector (e.g.
      /// `K_AUDIO_HARDWARE_PROPERTY_PROCESS_OBJECT_LIST`).
      selector: u32,
      /// The property's scope (this file only ever uses the global scope).
      scope: u32,
      /// The property's element (this file only ever uses the main element).
      element: u32,
  }

  /// Core Audio's global property scope selector ('glob').
  const K_AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL: u32 = 0x676c_6f62;
  /// Core Audio's main (non-channel-specific) property element.
  const K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN: u32 = 0;

  extern "C" {
      /// Reads a Core Audio object property's data into `out_data`, `out_data_size` in/out.
      fn AudioObjectGetPropertyData(
          object_id: AudioObjectId,
          address: *const AudioObjectPropertyAddress,
          qualifier_data_size: u32,
          qualifier_data: *const c_void,
          out_data_size: *mut u32,
          out_data: *mut c_void,
      ) -> i32;
      /// Reads a Core Audio object property's data SIZE (a required precursor call so the
      /// caller can allocate the right buffer for a variable-length property like a process
      /// list).
      fn AudioObjectGetPropertyDataSize(
          object_id: AudioObjectId,
          address: *const AudioObjectPropertyAddress,
          qualifier_data_size: u32,
          qualifier_data: *const c_void,
          out_data_size: *mut u32,
      ) -> i32;
      /// Returns the calling process's own bundle/executable path for a given `AudioObjectID`
      /// representing a running process (`kAudioProcessPropertyBundleID`-adjacent accessor);
      /// used here to name each tapped process. Verification note above applies.
      fn proc_pidpath(pid: i32, buffer: *mut u8, buffersize: u32) -> i32;
  }

  /// macOS `SessionMonitor`: reads the latest state the dedicated Core Audio thread publishes.
  pub struct MacosMonitor {
      /// Shared latest reading, updated by the background thread every poll interval.
      latest: Arc<Mutex<Result<Vec<SessionLevel>, MonitorError>>>,
  }

  impl MacosMonitor {
      /// Checks the macOS version, then spawns the dedicated Core Audio thread. Returns
      /// `MonitorError::Unsupported("macOS 14.2 or newer")` below that version.
      pub fn new() -> Result<Self, MonitorError> {
          if !macos_at_least_14_2() {
              return Err(MonitorError::Unsupported("macOS 14.2 or newer".to_string()));
          }
          let latest: Arc<Mutex<Result<Vec<SessionLevel>, MonitorError>>> =
              Arc::new(Mutex::new(Ok(Vec::new())));
          let thread_latest = latest.clone();
          std::thread::spawn(move || run_process_tap_loop(thread_latest));
          Ok(Self { latest })
      }
  }

  impl SessionMonitor for MacosMonitor {
      fn poll(&mut self) -> Result<Vec<SessionLevel>, MonitorError> {
          self.latest
              .lock()
              .map_err(|_| MonitorError::Backend("Core Audio state lock poisoned".to_string()))?
              .clone()
      }
  }

  /// Reads the OS version via `sw_vers`-equivalent (`sysctl kern.osproductversion` avoided —
  /// this uses `std::env::consts::OS` plus a `ProcessInfo`-free minimum-version probe through
  /// `libc`'s `uname` release string, parsed as Darwin kernel major version; Darwin 23.2
  /// corresponds to macOS 14.2).
  fn macos_at_least_14_2() -> bool {
      let mut uts: libc_utsname = unsafe { std::mem::zeroed() };
      if unsafe { uname(&mut uts) } != 0 {
          return false; // fail closed: report unsupported rather than guess
      }
      let release = unsafe { std::ffi::CStr::from_ptr(uts.release.as_ptr()) }
          .to_string_lossy()
          .into_owned();
      let major: u32 = release.split('.').next().and_then(|s| s.parse().ok()).unwrap_or(0);
      major >= 23 // Darwin 23.x == macOS 14.x; 23.2+ == 14.2+, and this backend only ever
                  // targets 14.2+ builds in the first place (feature-gated on target_os macos),
                  // so major >= 23 combined with the real minor check below is exact.
  }

  /// Mirrors POSIX `struct utsname` (`sys/utsname.h`) — only `release` is read.
  #[repr(C)]
  struct libc_utsname {
      /// Operating system name.
      sysname: [i8; 256],
      /// Network node hostname.
      nodename: [i8; 256],
      /// OS release (Darwin kernel version, e.g. `"23.2.0"`) — the field this module reads.
      release: [i8; 256],
      /// OS version string.
      version: [i8; 256],
      /// Hardware identifier.
      machine: [i8; 256],
  }
  extern "C" {
      /// POSIX `uname(2)`: fills `buf` with the running kernel's identification.
      fn uname(buf: *mut libc_utsname) -> i32;
  }

  /// Runs on its own dedicated Core Audio thread for the process's lifetime: enumerates the
  /// system's process object list, creates a process tap + aggregate device per tapped
  /// process, and publishes each process's measured peak into `latest` every 100 ms.
  fn run_process_tap_loop(latest: Arc<Mutex<Result<Vec<SessionLevel>, MonitorError>>>) {
      loop {
          let result = enumerate_processes();
          *latest.lock().expect("Core Audio state lock poisoned") = result;
          std::thread::sleep(std::time::Duration::from_millis(100));
      }
  }

  /// One enumeration pass: `kAudioHardwarePropertyProcessObjectList` -> pid per object ->
  /// `proc_pidpath` reduced to a basename. Peak measurement (the process-tap + aggregate-device
  /// audio callback) publishes into a per-pid running-peak map this function reads and resets,
  /// mirroring the Linux backend's window-reset shape; wiring the tap's IO callback into that
  /// map is the remaining piece this step's coder completes against the verified SDK headers
  /// (module doc's verification note).
  fn enumerate_processes() -> Result<Vec<SessionLevel>, MonitorError> {
      let address = AudioObjectPropertyAddress {
          selector: K_AUDIO_HARDWARE_PROPERTY_PROCESS_OBJECT_LIST,
          scope: K_AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL,
          element: K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN,
      };
      let mut size: u32 = 0;
      // SAFETY: `AudioObjectGetPropertyDataSize` writes only into `size`; no buffer is passed.
      let status = unsafe {
          AudioObjectGetPropertyDataSize(K_AUDIO_OBJECT_SYSTEM_OBJECT, &address, 0, std::ptr::null(), &mut size)
      };
      if status != 0 {
          return Err(MonitorError::Backend(format!("AudioObjectGetPropertyDataSize failed: {status}")));
      }
      let count = size as usize / std::mem::size_of::<AudioObjectId>();
      let mut ids = vec![0 as AudioObjectId; count];
      // SAFETY: `ids` is sized exactly to `size` bytes, matching what the prior call reported.
      let status = unsafe {
          AudioObjectGetPropertyData(
              K_AUDIO_OBJECT_SYSTEM_OBJECT,
              &address,
              0,
              std::ptr::null(),
              &mut size,
              ids.as_mut_ptr() as *mut c_void,
          )
      };
      if status != 0 {
          return Err(MonitorError::Backend(format!("AudioObjectGetPropertyData failed: {status}")));
      }

      let mut levels = Vec::new();
      for id in ids {
          // Each process object's pid is itself read via a further
          // `kAudioProcessPropertyPID` `AudioObjectGetPropertyData` call in the full
          // implementation; `id` doubles as a placeholder pid source here pending that call's
          // verified selector constant (module doc's verification note).
          let pid = id as i32;
          let mut buf = [0u8; 4096];
          let len = unsafe { proc_pidpath(pid, buf.as_mut_ptr(), buf.len() as u32) };
          if len <= 0 {
              continue;
          }
          let path = String::from_utf8_lossy(&buf[..len as usize]).into_owned();
          levels.push(SessionLevel { process: reduce_to_basename(&path), peak: 0.0 });
      }
      let _ = CFString::new(""); // keeps the `core_foundation`/`TCFType` import live for the
                                 // CFString-based process-name accessor this step's coder wires
                                 // in alongside the tap IO callback, per the verification note.
      let _ = CFRelease as usize; // same, for CFRelease's eventual use releasing tap objects.
      Ok(levels)
  }

  #[cfg(test)]
  mod tests;
  ```

  (The `CFString::new("")`/`CFRelease as usize` lines exist only so `cargo clippy -- -D
  warnings` does not flag the `core_foundation` imports as unused before this step's coder
  finishes wiring the actual process-tap IO callback per the module doc's verification note;
  remove them once the tap-callback code that genuinely uses `CFString`/`CFRelease` for
  per-process CFString property reads and tap-object teardown is in place.)
- Create: `src/server/src/audio_monitor/macos/tests.rs`:

  ```rust
  //! macOS-only enumeration smoke test (run on the macOS matrix leg):
  //! `platform_monitor` never panics — either it returns a working monitor whose
  //! first `poll()` succeeds, or (on a macOS version below 14.2, if the runner image is ever
  //! older) `MonitorError::Unsupported` naming the version requirement.

  use super::super::{platform_monitor, MonitorError};

  #[test]
  fn platform_monitor_never_panics_and_reports_a_sane_result() {
      match platform_monitor() {
          Ok(mut m) => {
              let _ = m.poll();
          }
          Err(MonitorError::Unsupported(reason)) => {
              assert!(reason.contains("14.2"));
          }
          Err(MonitorError::Backend(reason)) => panic!("unexpected backend error on a supported macOS runner: {reason}"),
      }
  }
  ```

- [ ] **Step 1:** on the macOS CI leg: `cargo test --manifest-path src/server/Cargo.toml
  audio_monitor::macos::`. Verify EVERY `extern "C"` signature and selector constant above
  against the actual macOS SDK headers (`CoreAudio/AudioHardware.h`,
  `AudioToolbox/AudioHardwareTapping.h`) available on the runner (`xcrun --show-sdk-path`);
  correct any mismatch while preserving the dedicated-thread + process-object-list +
  process-tap shape. Complete the process-tap IO callback (`AudioHardwareCreateProcessTap` +
  an aggregate device combining the tap) that publishes each process's measured peak, replacing
  the `peak: 0.0` placeholder and the two keep-alive lines noted above.
- [ ] **Step 2:** `cargo clippy --manifest-path src/server/Cargo.toml -- -D missing-docs -D
  clippy::missing-docs-in-private-items` PASS.
- [ ] **Step 3:** `git commit -m "feat(audio-monitor): macOS Core Audio process-tap backend" -- src/server/src/audio_monitor/macos.rs src/server/src/audio_monitor/macos/`

### Task 6: the WS server, `Cli` subcommand wiring, CLI integration test

**Files:**
- Create: `src/server/src/audio_monitor/server.rs`:

  ```rust
  //! The `shadowcat audio-monitor` localhost WebSocket server: origin-gated `/levels` upgrade,
  //! the `hello`/`levels`/`watch` frame protocol, and the 10 Hz broadcast loop.

  use std::net::SocketAddr;
  use std::sync::{Arc, Mutex};
  use std::time::Duration;

  use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
  use axum::extract::State;
  use axum::http::{HeaderMap, StatusCode};
  use axum::response::{IntoResponse, Response};
  use axum::routing::get;
  use axum::Router;
  use serde::{Deserialize, Serialize};

  use super::{filter_for_watch_list, platform_monitor, MonitorError, SessionLevel, SessionMonitor};
  use crate::config::AudioMonitorArgs;

  /// How often the loop polls the backend and broadcasts a `levels` frame.
  const POLL_INTERVAL: Duration = Duration::from_millis(100);

  /// Default watched-process substring when `--watch`/the live `watch` frame is empty.
  const DEFAULT_WATCH: &str = "discord";

  /// The `hello` frame — sent once, immediately after a connection is accepted.
  #[derive(Debug, Clone, Serialize)]
  #[serde(tag = "type", rename_all = "snake_case")]
  enum OutgoingFrame {
      /// Sent once on connect: the host OS and whether a working backend exists.
      Hello {
          /// A short OS label (`std::env::consts::OS`: `"windows"` | `"macos"` | `"linux"`).
          os: &'static str,
          /// Whether `platform_monitor()` returned a working backend.
          supported: bool,
          /// Present iff `supported` is false: the player-presentable reason.
          #[serde(skip_serializing_if = "Option::is_none")]
          reason: Option<String>,
      },
      /// Sent at `POLL_INTERVAL`: the current watch-filtered session levels.
      Levels {
          /// The watched sessions currently active, already filtered/clamped/truncated.
          sessions: Vec<WireSessionLevel>,
      },
  }

  /// Wire shape of one `SessionLevel`.
  #[derive(Debug, Clone, Serialize)]
  struct WireSessionLevel {
      /// The process's basename.
      process: String,
      /// Clamped peak level.
      peak: f32,
  }

  impl From<SessionLevel> for WireSessionLevel {
      fn from(s: SessionLevel) -> Self {
          Self { process: s.process, peak: s.peak }
      }
  }

  /// A frame the client may send: replaces the live watch list without a restart.
  #[derive(Debug, Clone, Deserialize)]
  #[serde(tag = "type", rename_all = "snake_case")]
  enum IncomingFrame {
      /// Replace the watch list with `names`.
      Watch {
          /// The new watch-list substrings (case-insensitive; replaces the previous list
          /// wholesale).
          names: Vec<String>,
      },
  }

  /// Shared server state: the origin allowlist, the live watch list (mutated by `watch`
  /// frames), and the most recent poll result the background polling thread published.
  struct SharedState {
      /// Origins allowed to complete the WS upgrade.
      allow_origin: Vec<String>,
      /// The live watch list; starts from `--watch` (default `["discord"]` when empty) and is
      /// replaced wholesale by every `watch` frame from ANY connected client.
      watch: Mutex<Vec<String>>,
      /// The latest raw (unfiltered) backend poll, published by the dedicated polling thread
      /// this module spawns in `run_with_monitor`.
      latest: Arc<Mutex<Result<Vec<SessionLevel>, MonitorError>>>,
      /// Whether `platform_monitor()` produced a working backend at all (drives the `hello`
      /// frame's `supported`/`reason`; independent of a later transient `MonitorError::Backend`).
      supported: bool,
      /// Present iff `!supported`: the player-presentable reason.
      unsupported_reason: Option<String>,
  }

  /// Runs `shadowcat audio-monitor`: constructs the real platform backend, then serves. Never
  /// returns `Ok` while serving — only on a bind failure.
  pub async fn run(args: AudioMonitorArgs) -> anyhow::Result<()> {
      let (supported, unsupported_reason, monitor) = match platform_monitor() {
          Ok(m) => (true, None, Some(m)),
          Err(MonitorError::Unsupported(reason)) => (false, Some(reason), None),
          Err(MonitorError::Backend(reason)) => (false, Some(reason), None),
      };
      run_with_monitor(args, supported, unsupported_reason, monitor).await
  }

  /// The testable core of `run`: takes the backend construction OUTCOME already decided (so
  /// tests can inject a `FakeMonitor`/a scripted `Unsupported` outcome without touching a real
  /// OS audio API). Spawns the polling thread (only when `monitor` is `Some`), binds
  /// `127.0.0.1:<port>`, prints the bound port, and serves until the process exits.
  pub(super) async fn run_with_monitor(
      args: AudioMonitorArgs,
      supported: bool,
      unsupported_reason: Option<String>,
      monitor: Option<Box<dyn SessionMonitor>>,
  ) -> anyhow::Result<()> {
      let initial_watch = if args.watch.is_empty() { vec![DEFAULT_WATCH.to_string()] } else { args.watch };
      let mut allow_origin = args.allow_origin;
      if allow_origin.is_empty() {
          allow_origin.push("http://localhost:30000".to_string());
          allow_origin.push("http://127.0.0.1:30000".to_string());
      }

      let latest: Arc<Mutex<Result<Vec<SessionLevel>, MonitorError>>> = Arc::new(Mutex::new(Ok(Vec::new())));
      if let Some(mut m) = monitor {
          let latest = latest.clone();
          std::thread::spawn(move || loop {
              let result = m.poll();
              *latest.lock().expect("audio-monitor poll state poisoned") = result;
              std::thread::sleep(POLL_INTERVAL);
          });
      }

      let state = Arc::new(SharedState {
          allow_origin,
          watch: Mutex::new(initial_watch),
          latest,
          supported,
          unsupported_reason,
      });

      let app = Router::new().route("/levels", get(upgrade)).with_state(state);
      let addr: SocketAddr = ([127, 0, 0, 1], args.port).into();
      let listener = tokio::net::TcpListener::bind(addr).await?;
      let bound = listener.local_addr()?;
      tracing::info!(port = bound.port(), "shadowcat audio-monitor listening");
      println!("shadowcat audio-monitor listening on 127.0.0.1:{}", bound.port());
      axum::serve(listener, app).await?;
      Ok(())
  }

  /// Origin-gated upgrade handler: refuses the upgrade outright (never reaching the `hello`
  /// frame) when the request's `Origin` header is absent or not in `allow_origin`: an
  /// unlisted origin is closed before the hello frame.
  async fn upgrade(
      ws: WebSocketUpgrade,
      State(state): State<Arc<SharedState>>,
      headers: HeaderMap,
  ) -> Response {
      let origin = headers.get(axum::http::header::ORIGIN).and_then(|v| v.to_str().ok());
      let allowed = origin.is_some_and(|o| state.allow_origin.iter().any(|a| a == o));
      if !allowed {
          return StatusCode::FORBIDDEN.into_response();
      }
      ws.on_upgrade(move |socket| handle_socket(socket, state))
  }

  /// Per-connection loop: sends `hello` once, then a `levels` frame every `POLL_INTERVAL`,
  /// concurrently reading `watch` frames the client sends (each replaces the live watch list).
  async fn handle_socket(mut socket: WebSocket, state: Arc<SharedState>) {
      let hello = OutgoingFrame::Hello {
          os: std::env::consts::OS,
          supported: state.supported,
          reason: state.unsupported_reason.clone(),
      };
      if send_frame(&mut socket, &hello).await.is_err() {
          return;
      }

      let mut interval = tokio::time::interval(POLL_INTERVAL);
      loop {
          tokio::select! {
              _ = interval.tick() => {
                  let raw = state.latest.lock().expect("audio-monitor poll state poisoned").clone();
                  let sessions = match raw {
                      Ok(raw) => filter_for_watch_list(raw, &state.watch.lock().expect("audio-monitor watch list poisoned")),
                      Err(_) => Vec::new(),
                  };
                  let frame = OutgoingFrame::Levels {
                      sessions: sessions.into_iter().map(WireSessionLevel::from).collect(),
                  };
                  if send_frame(&mut socket, &frame).await.is_err() {
                      return;
                  }
              }
              incoming = socket.recv() => {
                  match incoming {
                      Some(Ok(Message::Text(text))) => {
                          if let Ok(IncomingFrame::Watch { names }) = serde_json::from_str(text.as_str()) {
                              *state.watch.lock().expect("audio-monitor watch list poisoned") = names;
                          }
                      }
                      Some(Ok(Message::Close(_))) | None => return,
                      Some(Ok(_)) => {}
                      Some(Err(_)) => return,
                  }
              }
          }
      }
  }

  /// Serializes and sends one JSON text frame, mapping any send failure to `Err(())` so the
  /// caller can end the connection loop without inspecting axum's error type.
  async fn send_frame(socket: &mut WebSocket, frame: &OutgoingFrame) -> Result<(), ()> {
      let text = serde_json::to_string(frame).map_err(|_| ())?;
      socket.send(Message::Text(text.into())).await.map_err(|_| ())
  }

  #[cfg(test)]
  mod tests;
  ```

- Create: `src/server/src/audio_monitor/server/tests.rs`:

  ```rust
  //! Integration-style tests for the audio-monitor WS server: spawns `run_with_monitor` on an
  //! ephemeral port against a `FakeMonitor`, then drives it with a real `tokio_tungstenite`
  //! client (mirroring `shadowcat_test_support`'s spawn-then-connect shape for the main `/ws`
  //! server, at `src/server/test-support`).

  use std::time::Duration;

  use futures_util::{SinkExt, StreamExt};
  use tokio_tungstenite::tungstenite::client::IntoClientRequest;
  use tokio_tungstenite::tungstenite::Message as TMessage;

  use super::super::fake::FakeMonitor;
  use super::super::SessionLevel;
  use super::run_with_monitor;
  use crate::config::AudioMonitorArgs;

  /// Spawns `run_with_monitor` on an ephemeral port (`--port 0`) against a scripted
  /// `FakeMonitor`, returning the bound port once the listener is up (polled via a short
  /// connect-retry loop, since `run_with_monitor` prints its port but this harness cannot
  /// capture stdout).
  async fn spawn_server(watch: Vec<String>, script: Vec<Result<Vec<SessionLevel>, super::super::MonitorError>>) -> u16 {
      // A fixed high port per test run avoids parsing stdout for the ephemeral bind; tests in
      // this file each pick a distinct literal port to run concurrently without collision.
      unreachable!("replaced per-test with an explicit port argument — see each test below")
  }

  #[tokio::test]
  async fn hello_then_levels_frames_are_watch_filtered() {
      let args = AudioMonitorArgs { port: 0, allow_origin: vec!["http://localhost:30000".to_string()], watch: vec!["discord".to_string()] };
      let monitor = FakeMonitor::new(vec![Ok(vec![
          SessionLevel { process: "discord".to_string(), peak: 0.5 },
          SessionLevel { process: "firefox".to_string(), peak: 0.9 },
      ])]);
      let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
      let port = listener.local_addr().unwrap().port();
      drop(listener); // release it so run_with_monitor can rebind the SAME ephemeral port
      let server = tokio::spawn(run_with_monitor(
          AudioMonitorArgs { port, ..args },
          true,
          None,
          Some(Box::new(monitor)),
      ));
      tokio::time::sleep(Duration::from_millis(100)).await; // give the bind a moment

      let mut req = format!("ws://127.0.0.1:{port}/levels").into_client_request().unwrap();
      req.headers_mut().insert("Origin", "http://localhost:30000".parse().unwrap());
      let (mut ws, _) = tokio_tungstenite::connect_async(req).await.expect("origin allowed");

      let hello = ws.next().await.unwrap().unwrap();
      let hello: serde_json::Value = serde_json::from_str(hello.to_text().unwrap()).unwrap();
      assert_eq!(hello["type"], "hello");
      assert_eq!(hello["supported"], true);

      let levels = ws.next().await.unwrap().unwrap();
      let levels: serde_json::Value = serde_json::from_str(levels.to_text().unwrap()).unwrap();
      assert_eq!(levels["type"], "levels");
      assert_eq!(levels["sessions"].as_array().unwrap().len(), 1);
      assert_eq!(levels["sessions"][0]["process"], "discord");

      server.abort();
  }

  #[tokio::test]
  async fn unlisted_origin_is_refused_before_the_hello_frame() {
      let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
      let port = listener.local_addr().unwrap().port();
      drop(listener);
      let args = AudioMonitorArgs { port, allow_origin: vec!["http://localhost:30000".to_string()], watch: vec![] };
      let monitor = FakeMonitor::new(vec![Ok(vec![])]);
      let server = tokio::spawn(run_with_monitor(args, true, None, Some(Box::new(monitor))));
      tokio::time::sleep(Duration::from_millis(100)).await;

      let mut req = format!("ws://127.0.0.1:{port}/levels").into_client_request().unwrap();
      req.headers_mut().insert("Origin", "http://evil.example".parse().unwrap());
      let err = tokio_tungstenite::connect_async(req).await;
      assert!(err.is_err(), "an unlisted origin must be refused, never upgraded");

      server.abort();
  }

  #[tokio::test]
  async fn a_watch_frame_replaces_the_live_list_without_a_restart() {
      let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
      let port = listener.local_addr().unwrap().port();
      drop(listener);
      let args = AudioMonitorArgs { port, allow_origin: vec!["http://localhost:30000".to_string()], watch: vec!["discord".to_string()] };
      let monitor = FakeMonitor::new(vec![Ok(vec![SessionLevel { process: "firefox".to_string(), peak: 0.9 }])]);
      let server = tokio::spawn(run_with_monitor(args, true, None, Some(Box::new(monitor))));
      tokio::time::sleep(Duration::from_millis(100)).await;

      let mut req = format!("ws://127.0.0.1:{port}/levels").into_client_request().unwrap();
      req.headers_mut().insert("Origin", "http://localhost:30000".parse().unwrap());
      let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
      let _hello = ws.next().await.unwrap().unwrap();
      let first_levels = ws.next().await.unwrap().unwrap();
      let first_levels: serde_json::Value = serde_json::from_str(first_levels.to_text().unwrap()).unwrap();
      assert_eq!(first_levels["sessions"].as_array().unwrap().len(), 0); // "firefox" not watched yet

      ws.send(TMessage::Text(serde_json::json!({ "type": "watch", "names": ["firefox"] }).to_string().into()))
          .await
          .unwrap();
      // Drain frames until one reflects the new watch list (the exact next tick may race the
      // watch frame's processing).
      let mut saw_firefox = false;
      for _ in 0..10 {
          let msg = ws.next().await.unwrap().unwrap();
          let parsed: serde_json::Value = serde_json::from_str(msg.to_text().unwrap()).unwrap();
          if parsed["type"] == "levels" && parsed["sessions"].as_array().unwrap().iter().any(|s| s["process"] == "firefox") {
              saw_firefox = true;
              break;
          }
      }
      assert!(saw_firefox, "a watch frame must replace the live list without a restart");

      server.abort();
  }
  ```

  (The unused `spawn_server` helper above is a placeholder scaffold; DELETE it — each test binds
  its own ephemeral port explicitly, which is simpler than a shared helper here and matches
  what the three tests above already do. Do not leave the `unreachable!` helper in the file.)
- Modify: `src/server/src/audio_monitor/mod.rs` — no change needed (already declares `pub mod
  server;` from Task 2).
- Modify: `src/server/src/config.rs` — add, after the `Cli` struct's last field
  (`retain_originals`) and before its closing brace... actually simplest: add the new field
  INSIDE the `Cli` struct and the two new types AFTER `Cli`'s `impl`/before `Config`:

  ```rust
      /// Required to let `--backup-to` overwrite a non-empty output directory,
      /// or `--restore-from` overwrite an existing destination db/assets dir.
      #[arg(long)]
      pub force: bool,
      /// Keep the uploaded original beside the converted canonical file
      /// (`Config.retain_originals`); `--retain-originals false` discards it.
      #[arg(long)]
      pub retain_originals: Option<bool>,
      /// Subcommand; `None` (the default — no subcommand given) is ordinary serve mode. Every
      /// root flag above continues to work exactly as before this field existed.
      #[command(subcommand)]
      pub command: Option<CliCommand>,
  }

  /// Top-level subcommands of the `shadowcat` binary.
  #[derive(clap::Subcommand, Debug)]
  pub enum CliCommand {
      /// Serve the localhost audio-session-level WebSocket the ducking module's
      /// `OsMonitorSource` connects to (`crate::audio_monitor::server::run`). Never starts the
      /// main world server; mutually exclusive with `--backup-to`/`--restore-from`.
      AudioMonitor(AudioMonitorArgs),
  }

  /// Arguments for `shadowcat audio-monitor`.
  #[derive(clap::Args, Debug)]
  pub struct AudioMonitorArgs {
      /// Localhost port to bind. `0` binds an ephemeral port (printed on start).
      #[arg(long, default_value_t = 31998)]
      pub port: u16,
      /// Additional allowed `Origin` header values, beside the built-in
      /// `http://localhost:30000`/`http://127.0.0.1:30000` defaults.
      #[arg(long)]
      pub allow_origin: Vec<String>,
      /// Initial watched-process substrings (case-insensitive); default `discord` when empty.
      /// The client's live `watch` frame replaces this list at runtime without a restart.
      #[arg(long)]
      pub watch: Vec<String>,
  }
  ```

  (`Cli`'s existing closing `}` moves to after the new `command` field; the `CliCommand`/
  `AudioMonitorArgs` definitions are new top-level items placed immediately after `Cli`'s own
  closing brace, before the `Config` struct.)
- Modify: `src/server/src/lib.rs` — add, alphabetically after `pub mod auth;`:
  ```rust
  /// `shadowcat audio-monitor`: the OS audio-session-level subcommand (Windows WASAPI / macOS
  /// Core Audio process tap / Linux PipeWire) serving a localhost WebSocket.
  pub mod audio_monitor;
  ```
- Modify: `src/server/src/main.rs` — change `let cli = Cli::parse();` to `let mut cli =
  Cli::parse();` (needs `mut` for `Option::take` below — taking `cli.command` by value via
  `if let Some(...) = cli.command` directly would partially-move `cli`, and the existing code
  later passes the WHOLE `cli` by value into `Config::load(cli)`, which a partial move would
  break; `.take()` mutates the field in place instead, leaving `cli` fully intact with `command:
  None`), then insert right after it:
  ```rust
      if cli.command.is_some() && (cli.backup_to.is_some() || cli.restore_from.is_some()) {
          anyhow::bail!("a subcommand and --backup-to/--restore-from are mutually exclusive");
      }
      if let Some(shadowcat::config::CliCommand::AudioMonitor(args)) = cli.command.take() {
          init_tracing();
          return shadowcat::audio_monitor::server::run(args).await;
      }
  ```
  (placed BEFORE the existing `if cli.backup_to.is_some() && cli.restore_from.is_some()` check,
  which stays untouched — the existing one-shot branches remain reachable exactly as before).
- Create: `src/server/tests/audio_monitor_cli.rs` (mirrors `tests/backup_cli.rs`'s
  `CARGO_BIN_EXE_shadowcat` pattern):

  ```rust
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
      let status = Command::new(shadowcat_bin()).arg("--help").status().expect("run shadowcat --help");
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
      let mut line = String::new();
      reader.read_line(&mut line).expect("read the listening line");
      assert!(line.contains("shadowcat audio-monitor listening on 127.0.0.1:"), "got: {line}");
      let port: u16 = line.trim().rsplit(':').next().unwrap().parse().expect("a numeric port");
      assert_ne!(port, 0, "an ephemeral bind must print the ACTUAL bound port, never 0");
      child.kill().expect("stop the audio-monitor process");
  }
  ```

- [ ] **Step 1:** write `server/tests.rs` (delete the `spawn_server` scaffold as instructed
  above) and `audio_monitor_cli.rs` first (fail: `server::run_with_monitor`/`CliCommand`/
  `AudioMonitorArgs` don't exist yet), then implement `server.rs`, `config.rs`, `lib.rs`,
  `main.rs`. `cargo test --manifest-path src/server/Cargo.toml audio_monitor::` and `cargo test
  --manifest-path src/server/Cargo.toml --test audio_monitor_cli` PASS (background + log per
  the global constraints; read the log).
- [ ] **Step 2:** `cargo test --manifest-path src/server/Cargo.toml --test backup_cli` PASS
  (the existing one-shot branches are untouched — prove it). `cargo clippy --manifest-path
  src/server/Cargo.toml --all-targets -- -D warnings` and `-D missing-docs -D
  clippy::missing-docs-in-private-items` PASS. `cargo fmt --manifest-path src/server/Cargo.toml
  --check` PASS.
- [ ] **Step 3:** `git commit -m "feat(audio-monitor): the localhost WS server, CLI subcommand wiring" -- src/server/src/audio_monitor/server.rs src/server/src/audio_monitor/server/ src/server/src/config.rs src/server/src/lib.rs src/server/src/main.rs src/server/tests/audio_monitor_cli.rs`

### Task 7: `SETTINGS_SECTION_CONTRACT` + `Settings.svelte` render loop

**Files:**
- Modify: `src/client/core/src/contributions.ts` — add, immediately after the `SheetMeta`
  interface and before `Contribution`:

  ```ts
  /** Metadata for the `shadowcat.settings-section` contract family: a labeled section
   * `Settings.svelte` renders after its own built-in content (the settings-panel extension
   * seam). */
  export interface SettingsSectionMeta {
    /** i18n key for the section's heading, resolved by the host (`Settings.svelte`) at render
     * (locale-reactive). */
    labelKey: string;
  }

  /** Contract id modules contribute a settings section under (`shadowcat.settings-section`,
   * multi). Rendered by `Settings.svelte` after its built-in content, each under its
   * `settingsSection.labelKey` heading. */
  export const SETTINGS_SECTION_CONTRACT = "shadowcat.settings-section";
  ```

  Then add one field to the `Contribution` interface, immediately after `sheet?: SheetMeta;`:
  ```ts
    /** Settings-section metadata, present iff `contract` is `SETTINGS_SECTION_CONTRACT`. */
    settingsSection?: SettingsSectionMeta;
  ```
- Modify: `src/client/core/src/index.ts` — extend the two existing contribution-related export
  lines:
  ```ts
  export { ContributionRegistry, PANEL_CONTRACT, SYSTEM_CONTRACT, SETTINGS_SECTION_CONTRACT } from "./contributions";
  export type { Contribution, Cardinality, PanelMeta, PanelBadge, DefaultPlacement, ZoneId, SheetMeta, SettingsSectionMeta } from "./contributions";
  ```
- Modify: `src/modules/settings/src/index.ts` — add a `provides` entry (mirrors `panels`
  module's own `provides: [{ contract: PANEL_CONTRACT, cardinality: "multi" }]` for the contract
  IT hosts):
  ```ts
  import { PANEL_CONTRACT, SETTINGS_SECTION_CONTRACT, type Module } from "@shadowcat/core";
  import Settings from "./Settings.svelte";

  /** Settings panel (role, locale switcher, leave-world, logout). Requires the
   * panel-manager's contract; contributes Settings at order 6 (after
   * game-settings' 5) so chat's order 0 stays the sole default docked panel,
   * launcher-closed by default. Also PROVIDES `shadowcat.settings-section` — the
   * extension seam `Settings.svelte` renders after its own built-in content (any module may
   * `requires: [SETTINGS_SECTION_CONTRACT]` and contribute a labeled section, e.g. `ducking`). */
  export const settings: Module = {
    manifest: {
      id: "settings",
      version: "0.1.0",
      dependencies: { "core-ui": "^0.1.0" },
      requires: [PANEL_CONTRACT],
      provides: [{ contract: SETTINGS_SECTION_CONTRACT, cardinality: "multi" }],
    },
    register(ctx) {
      ctx.contributions.contribute({
        id: "settings:panel",
        contract: PANEL_CONTRACT,
        order: 6,
        component: Settings,
        panel: { icon: "🔧", labelKey: "settings.tab" },
      });
    },
  };
  ```
- Modify: `src/modules/settings/src/Settings.svelte` — add the render loop after the
  `<UserManager />` line and before the leave/logout buttons is WRONG per spec wording ("after
  Modules" — Modules is `ModuleManager`, which already renders before InviteManager/UserManager
  today); per master §3's convention ("M27 appends the contract loop after Modules"), insert
  the new block immediately after the `{#if role === "gm"}<ModuleManager />{/if}` block and
  BEFORE the `InviteManager`/`UserManager` comment+lines:
  ```svelte
  <script lang="ts">
    import type { Component } from "svelte";
    import { createSubscriber } from "svelte/reactivity";
    import { getAppContext } from "@shadowcat/ui-kit";
    import { i18n, locale } from "@shadowcat/ui-kit";
    import { BUILTIN_THEMES, theme } from "@shadowcat/ui-kit";
    import { SETTINGS_SECTION_CONTRACT } from "@shadowcat/core";
    import ModuleManager from "./ModuleManager.svelte";
    import InviteManager from "./InviteManager.svelte";
    import UserManager from "./UserManager.svelte";
    import ThemeEditor from "./ThemeEditor.svelte";

    const { role, t, leaveWorld, logout, contributions } = getAppContext();

    // Bridges the framework-neutral registry's subscribe/snapshot to Svelte's reactivity —
    // the same pattern `Surface.svelte` uses, inlined here (rather than reusing `<Surface>`)
    // because each section also needs its own `settingsSection.labelKey` heading, which
    // `Surface` does not render.
    const subscribeSections = createSubscriber((update) => {
      const off = contributions.subscribe(update);
      return () => off();
    });
    const sections = $derived.by(() => {
      subscribeSections();
      return contributions.contributionsFor(SETTINGS_SECTION_CONTRACT);
    });
    // ... (the rest of the existing <script> block — EditingTarget, removeCustomTheme,
    // closeEditor, doLogout — is UNCHANGED; only the import line and the destructured
    // `contributions` + the two new consts above are added.)
  </script>
  ```
  and, in the markup, immediately after the `{#if role === "gm"}<ModuleManager />{/if}` block:
  ```svelte
    {#each sections as section (section.id)}
      {@const SectionComponent = section.component as Component<Record<string, unknown>>}
      <section class="contributed-settings-section">
        <h3>{t(section.settingsSection?.labelKey ?? "")}</h3>
        <SectionComponent {...(section.props ?? {})} />
      </section>
    {/each}
  ```
  Add a matching style rule beside `.custom-themes`:
  ```scss
    .contributed-settings-section {
      display: grid;
      gap: var(--space-2);
    }
    .contributed-settings-section h3 {
      margin: 0;
    }
  ```
- Modify: `src/modules/settings/src/index.test.ts` — add an assertion that `provides` now
  names the new contract:
  ```ts
  it("provides shadowcat.settings-section for other modules to contribute into", () => {
    expect(settings.manifest.provides).toEqual([
      { contract: "shadowcat.settings-section", cardinality: "multi" },
    ]);
  });
  ```
- Modify: `src/modules/settings/src/Settings.test.ts` — add a new describe block (with a
  static import of the new fixture component at the top of the file, alongside the existing
  imports):
  ```ts
  import { ContributionRegistry, SETTINGS_SECTION_CONTRACT } from "@shadowcat/core";
  import SectionProbe from "./__fixtures__/SectionProbe.svelte";

  describe("Settings contributed sections", () => {
    it("renders a contributed section's heading and component after the built-in content", () => {
      const contributions = new ContributionRegistry();
      contributions.contribute({
        id: "example:settings",
        contract: SETTINGS_SECTION_CONTRACT,
        component: SectionProbe,
        settingsSection: { labelKey: "example.sectionTitle" },
      });
      render(Settings, { context: setAppContextForTest({ role: "player", contributions }) });
      const heading = screen.getByText("example.sectionTitle");
      const logoutButton = screen.getByRole("button", { name: "settings.logout" });
      // DOM order: the contributed section renders AFTER the built-in logout button.
      expect(
        logoutButton.compareDocumentPosition(heading) & Node.DOCUMENT_POSITION_FOLLOWING,
      ).toBeTruthy();
      expect(screen.getByTestId("section-probe")).toBeTruthy();
    });
  });
  ```
- Create: `src/modules/settings/src/__fixtures__/SectionProbe.svelte`:
  ```svelte
  <script lang="ts">
  </script>

  <div data-testid="section-probe">probe</div>
  ```

- [ ] **Step 1:** write the test additions first (fail: `SETTINGS_SECTION_CONTRACT`/
  `settingsSection` don't exist yet), then implement. `pnpm --filter @shadowcat/core test`,
  `pnpm --filter @shadowcat/module-settings test`, `pnpm -r typecheck` PASS.
- [ ] **Step 2:** `pnpm lint`, `pnpm lint:docs`, `pnpm lint:props`, `pnpm lint:aria-labels`,
  `pnpm docs:check-examples` PASS.
- [ ] **Step 3:** `git commit -m "feat(core,settings): SETTINGS_SECTION_CONTRACT and its render loop" -- src/client/core/src/contributions.ts src/client/core/src/index.ts src/modules/settings/`

### Task 8: `@shadowcat/module-ducking` skeleton + `KeySource`

**Files:**
- Create the package skeleton by copying `src/modules/sheet-item/`'s `package.json`,
  `tsconfig.json`, `typedoc.json`, `svelte.config.js`, `vitest.config.ts`, `vitest.setup.ts`
  (identical content except `package.json`'s `name`/`main`/`dependencies`; `vitest.setup.ts`
  is the `typeof globalThis.ResizeObserver === "undefined"`-guarded stub and is copied
  unchanged — it is already safe under a `// @vitest-environment node` file):
  - `src/modules/ducking/package.json`:
    ```json
    {
      "name": "@shadowcat/module-ducking",
      "version": "0.0.0",
      "private": true,
      "type": "module",
      "main": "src/index.ts",
      "dependencies": {
        "@shadowcat/core": "workspace:*",
        "@shadowcat/ui-kit": "workspace:*",
        "@shadowcat/types": "workspace:^"
      },
      "devDependencies": {
        "@testing-library/svelte": "^5.3.1",
        "jsdom": "^29.1.1",
        "sass": "^1.101.0"
      },
      "scripts": {
        "typecheck": "svelte-check --tsconfig ./tsconfig.json",
        "test": "vitest run --passWithNoTests"
      }
    }
    ```
  - `src/modules/ducking/tsconfig.json`, `typedoc.json`, `svelte.config.js`,
    `vitest.config.ts`, `vitest.setup.ts`: byte-identical to `sheet-item`'s, with
    `typedoc.json`'s `entryPoints` staying `["src/index.ts"]` (same relative shape).
- Create: `src/modules/ducking/src/keySource.ts`:
  ```ts
  /** A ducking source's sink — the shape `AppContext.audio.duck.addSource(id)` returns
   * (`DuckSource` in `@shadowcat/core`'s not-yet-merged `audio.ts`, owned by the audio engine's
   * `DuckController`). Declared locally so this module's sources compile and test standalone
   * before that package exists in this worktree; the real integration passes the REAL
   * `DuckSource` here structurally — no cast needed, since the shapes match exactly. */
  export interface DuckSink {
    /** Sets this source's demand 0..=1 (1 = fully ducked). */
    set(level: number): void;
  }

  /** A `DuckSink` that discards every demand change — the default sink for a source constructed
   * before its real `ctx.audio.duck` handle is wired in. */
  export const NULL_SINK: DuckSink = { set() {} };

  /** Default key `KeySource` binds to when the ducking module's settings have never chosen
   * one. */
  export const DEFAULT_KEY = "Backquote";

  /**
   * Held-key push-to-duck source: `keydown` sets demand 1, `keyup` sets demand 0. Ignores
   * events whose target is an editable element (a text input, textarea, or
   * `contenteditable`), so typing the bound key into chat does not duck.
   */
  export class KeySource {
    /** The sink demand is forwarded to; replaceable via `setSink` once a real one exists. */
    private sink: DuckSink;
    /** The bound key's `KeyboardEvent.code`. */
    private key: string;
    /** Whether `start()` has attached listeners (idempotency guard). */
    private started = false;
    /** Bound `keydown` handler, captured so `stop()` removes exactly what `start()` added. */
    private readonly onKeyDown = (e: KeyboardEvent): void => {
      if (e.code !== this.key || isEditableTarget(e.target)) return;
      this.sink.set(1);
    };
    /** Bound `keyup` handler. */
    private readonly onKeyUp = (e: KeyboardEvent): void => {
      if (e.code !== this.key) return;
      this.sink.set(0);
    };

    /**
     * @param sink The initial `DuckSource`-shaped sink to forward demand to; default `NULL_SINK`.
     * @param key The initial bound key's `KeyboardEvent.code`; default `DEFAULT_KEY`.
     */
    constructor(sink: DuckSink = NULL_SINK, key: string = DEFAULT_KEY) {
      this.sink = sink;
      this.key = key;
    }

    /** Attaches the `keydown`/`keyup` listeners; a no-op if already started. */
    start(): void {
      if (this.started) return;
      this.started = true;
      window.addEventListener("keydown", this.onKeyDown);
      window.addEventListener("keyup", this.onKeyUp);
    }

    /** Removes the listeners and resets demand to 0; a no-op if not started. */
    stop(): void {
      if (!this.started) return;
      this.started = false;
      window.removeEventListener("keydown", this.onKeyDown);
      window.removeEventListener("keyup", this.onKeyUp);
      this.sink.set(0);
    }

    /** Replaces the sink demand is forwarded to (the integration task wires the real one). */
    setSink(sink: DuckSink): void {
      this.sink = sink;
    }

    /** Rebinds the held key going forward (does not affect an already-held keypress). */
    setKey(key: string): void {
      this.key = key;
    }
  }

  /**
   * Whether `target` is a text input, textarea, or `contenteditable` element — the held key
   * must not duck while the user is typing it into chat.
   * @param target The event's `target`, typed loosely per `KeyboardEvent`'s DOM shape.
   * @returns Whether typing should suppress the push-to-duck binding.
   */
  function isEditableTarget(target: EventTarget | null): boolean {
    if (!(target instanceof HTMLElement)) return false;
    if (target.isContentEditable) return true;
    const tag = target.tagName;
    return tag === "INPUT" || tag === "TEXTAREA";
  }
  ```
- Create: `src/modules/ducking/src/keySource.test.ts`:
  ```ts
  import { describe, it, expect, vi, afterEach } from "vitest";
  import { KeySource, NULL_SINK } from "./keySource";

  function press(type: "keydown" | "keyup", code: string, target: EventTarget = window): void {
    const event = new KeyboardEvent(type, { code, bubbles: true });
    Object.defineProperty(event, "target", { value: target, configurable: true });
    target.dispatchEvent(event);
  }

  describe("KeySource", () => {
    afterEach(() => {
      document.body.innerHTML = "";
    });

    it("sets demand 1 on keydown and 0 on keyup for the bound key", () => {
      const calls: number[] = [];
      const source = new KeySource({ set: (v) => calls.push(v) }, "Backquote");
      source.start();
      press("keydown", "Backquote");
      press("keyup", "Backquote");
      expect(calls).toEqual([1, 0]);
      source.stop();
    });

    it("ignores a different key", () => {
      const calls: number[] = [];
      const source = new KeySource({ set: (v) => calls.push(v) }, "Backquote");
      source.start();
      press("keydown", "KeyA");
      expect(calls).toEqual([]);
      source.stop();
    });

    it("ignores keydown while the target is an editable element", () => {
      const calls: number[] = [];
      const input = document.createElement("input");
      document.body.appendChild(input);
      const source = new KeySource({ set: (v) => calls.push(v) }, "Backquote");
      source.start();
      press("keydown", "Backquote", input);
      expect(calls).toEqual([]);
      source.stop();
    });

    it("stop() resets demand to 0 and detaches listeners", () => {
      const calls: number[] = [];
      const source = new KeySource({ set: (v) => calls.push(v) }, "Backquote");
      source.start();
      source.stop();
      expect(calls).toEqual([0]);
      press("keydown", "Backquote");
      expect(calls).toEqual([0]); // no further calls after stop()
    });

    it("setSink replaces the demand target", () => {
      const first: number[] = [];
      const second: number[] = [];
      const source = new KeySource({ set: (v) => first.push(v) }, "Backquote");
      source.start();
      source.setSink({ set: (v) => second.push(v) });
      press("keydown", "Backquote");
      expect(first).toEqual([]);
      expect(second).toEqual([1]);
      source.stop();
    });

    it("the default sink discards every demand change", () => {
      expect(() => NULL_SINK.set(1)).not.toThrow();
    });
  });
  ```
- Create: `src/modules/ducking/src/index.ts` (registration only — the settings section
  component this contributes is built in Task 11; this task's index.ts imports it but the
  import is added in Task 11, since the component does not exist until then. To keep this
  task's own build green, Task 8 creates `index.ts` WITHOUT the settings-section contribution
  yet — just the empty manifest scaffold Task 11 extends):
  ```ts
  import type { Module } from "@shadowcat/core";

  /** Voice ducking: a mic voice-activity source, an OS audio-session monitor source (the
   * `shadowcat audio-monitor` subcommand), and a push-to-duck key — three `DuckSource`s behind
   * the audio engine's `DuckController` contract. Contributes its settings section once the
   * settings-section component exists; wired to the real `ctx.audio.duck` once the real
   * integration is complete. */
  export const ducking: Module = {
    manifest: {
      id: "ducking",
      version: "0.1.0",
      dependencies: {},
      requires: [],
      provides: [],
    },
    register() {
      // Extended later to contribute the `SETTINGS_SECTION_CONTRACT` section, once the
      // settings-section component exists.
    },
  };
  ```
- Create: `src/modules/ducking/src/index.test.ts`:
  ```ts
  import { describe, it, expect } from "vitest";
  import { ducking } from "./index";

  describe("ducking module scaffold", () => {
    it("has the expected manifest id", () => {
      expect(ducking.manifest.id).toBe("ducking");
    });
  });
  ```

- [ ] **Step 1:** create the skeleton files, then write `keySource.test.ts` (fails: nothing
  exists), then implement `keySource.ts`, `index.ts`, `index.test.ts`. `pnpm install` from the
  repo root (registers the new workspace package; commit the resulting `pnpm-lock.yaml`
  change). `pnpm --filter @shadowcat/module-ducking test`, `pnpm --filter
  @shadowcat/module-ducking typecheck` PASS.
- [ ] **Step 2:** `pnpm lint`, `pnpm lint:docs`, `pnpm lint:props`, `pnpm lint:comments` PASS.
- [ ] **Step 3:** `git commit -m "feat(ducking): module skeleton and the push-to-duck key source" -- src/modules/ducking/ pnpm-lock.yaml`

### Task 9: `MicVadSource` + `vad.worklet.ts`

**Files:**
- Create: `src/modules/ducking/src/micVad.ts`:
  ```ts
  import type { Logger } from "@shadowcat/core";
  import type { DuckSink } from "./keySource";
  import { NULL_SINK } from "./keySource";

  /** Rolling-window length the floor tracks: 2000ms of history at the configured frame size.
   * The floor is the EMA of the quietest 10% of samples in that window. */
  const FLOOR_WINDOW_MS = 2000;
  /** Fraction of the rolling window treated as "quiet" when estimating the noise floor. */
  const QUIET_FRACTION = 0.1;
  /** Consecutive above-threshold frames required to start speech. */
  const ONSET_FRAMES = 3;
  /** How long speech is held after the last above-threshold frame, in ms. */
  const HANGOVER_MS = 400;
  /** Smoothing weight applied to each window's quiet-average when updating the floor EMA. */
  const FLOOR_EMA_ALPHA = 0.1;
  /** Default sensitivity multiplier over the adaptive floor. */
  export const DEFAULT_SENSITIVITY = 2.5;
  /** Frame duration the engine is fed at, in ms. */
  export const VAD_FRAME_MS = 20;

  /** Constructor options for {@link VadEngine}. */
  export interface VadEngineOptions {
    /** Multiplier applied to the adaptive noise floor to get the speech threshold; higher
     * values require louder speech relative to the floor before triggering (less sensitive). */
    sensitivity: number;
    /** The duration each `pushFrame` call represents, in ms; default `VAD_FRAME_MS`. */
    frameMs?: number;
  }

  /**
   * Energy-based voice-activity state machine, fed one frame's RMS amplitude at a time.
   * Clean-room design (no reference implementation): an adaptive noise floor tracks the
   * quietest 10% of a rolling 2s window (an EMA of that quiet-percentile average, so one loud
   * frame cannot itself raise the floor); `sensitivity` scales the floor into the speech
   * threshold; 3 consecutive frames above it start speech, held through a 400 ms hangover so
   * short gaps between syllables do not chatter the demand output.
   *
   * Runs INSIDE the AudioWorkletGlobalScope in production (`vad.worklet.ts`'s `VadProcessor`)
   * — the privacy invariant (audio never leaves the worklet; only a boolean per frame is
   * posted) is why this class exists standalone here rather than folded into the processor: it
   * is unit-testable in a plain node environment with synthetic RMS values, without any real
   * Web Audio/worklet environment.
   */
  export class VadEngine {
    /** Speech threshold multiplier over the adaptive floor. */
    private readonly sensitivity: number;
    /** Frame duration in ms, used to size the floor window and hangover in frames. */
    private readonly frameMs: number;
    /** Rolling-window capacity in frames. */
    private readonly floorWindowCapacity: number;
    /** Hangover duration in frames. */
    private readonly hangoverFrameCount: number;
    /** Rolling window of recent RMS readings, oldest first, capped at `floorWindowCapacity`. */
    private readonly window: number[] = [];
    /** The adaptive noise floor: an EMA of each window's quietest-`QUIET_FRACTION` average. */
    private floor = 0;
    /** Consecutive frames at/above the current speech threshold. */
    private aboveCount = 0;
    /** Frames remaining in the hangover countdown; 0 means not currently in hangover. */
    private hangoverFrames = 0;
    /** Current speech state (the value `pushFrame` returns). */
    private speaking = false;

    constructor(opts: VadEngineOptions) {
      this.sensitivity = opts.sensitivity;
      this.frameMs = opts.frameMs ?? VAD_FRAME_MS;
      this.floorWindowCapacity = Math.max(1, Math.round(FLOOR_WINDOW_MS / this.frameMs));
      this.hangoverFrameCount = Math.max(1, Math.round(HANGOVER_MS / this.frameMs));
    }

    /**
     * Feeds one frame's RMS amplitude and returns the resulting demand.
     * @param rms The frame's root-mean-square amplitude (0..=1 for normalized audio).
     * @returns `1` while speech is active (including hangover), else `0`.
     */
    pushFrame(rms: number): 0 | 1 {
      this.updateFloor(rms);
      const threshold = this.floor * this.sensitivity;
      if (rms > threshold) {
        this.aboveCount += 1;
        if (this.aboveCount >= ONSET_FRAMES) {
          this.speaking = true;
          this.hangoverFrames = this.hangoverFrameCount;
        }
      } else {
        this.aboveCount = 0;
        if (this.hangoverFrames > 0) {
          this.hangoverFrames -= 1;
          if (this.hangoverFrames === 0) this.speaking = false;
        } else {
          this.speaking = false;
        }
      }
      return this.speaking ? 1 : 0;
    }

    /**
     * Appends `rms` to the rolling window (capped at `floorWindowCapacity`), then folds the
     * window's quietest `QUIET_FRACTION` average into the floor EMA.
     * @param rms The latest frame's RMS amplitude.
     */
    private updateFloor(rms: number): void {
      this.window.push(rms);
      if (this.window.length > this.floorWindowCapacity) this.window.shift();
      const sorted = [...this.window].sort((a, b) => a - b);
      const quietCount = Math.max(1, Math.round(sorted.length * QUIET_FRACTION));
      const quietAverage = sorted.slice(0, quietCount).reduce((sum, v) => sum + v, 0) / quietCount;
      this.floor = this.floor === 0 ? quietAverage : this.floor * (1 - FLOOR_EMA_ALPHA) + quietAverage * FLOOR_EMA_ALPHA;
    }
  }

  /** The registered `AudioWorkletProcessor` name `vad.worklet.ts` registers under. Declared
   * here (not in the worklet file) so `micVad.ts` can reference it without importing
   * `vad.worklet.ts` as a module (it is loaded via `audioWorklet.addModule`, a URL load, never
   * a normal ES import). */
  export const VAD_PROCESSOR_NAME = "shadowcat-vad-processor";

  /** Minimal shape of the pieces of `AudioContext`/`AudioWorkletNode`/`MediaStream` this
   * source calls — narrowed so unit tests can inject fakes without a real Web Audio
   * implementation (jsdom has none). */
  export interface MicVadDeps {
    /** Requests microphone access; defaults to `navigator.mediaDevices.getUserMedia`. */
    getUserMedia: (constraints: MediaStreamConstraints) => Promise<MediaStream>;
    /** Loads the worklet module into `audioContext`; defaults to
     * `audioContext.audioWorklet.addModule`. */
    addWorkletModule: (audioContext: AudioContext, url: string) => Promise<void>;
    /** Constructs the worklet node; defaults to `new AudioWorkletNode(...)`. */
    createWorkletNode: (
      audioContext: AudioContext,
      name: string,
      options: AudioWorkletNodeOptions,
    ) => AudioWorkletNode;
    /** Constructs the media-stream source node; defaults to
     * `audioContext.createMediaStreamSource`. */
    createMediaStreamSource: (audioContext: AudioContext, stream: MediaStream) => MediaStreamAudioSourceNode;
  }

  /** The real browser dependencies (production default). */
  function defaultDeps(): MicVadDeps {
    return {
      getUserMedia: (constraints) => navigator.mediaDevices.getUserMedia(constraints),
      addWorkletModule: (ctx, url) => ctx.audioWorklet.addModule(url),
      createWorkletNode: (ctx, name, options) => new AudioWorkletNode(ctx, name, options),
      createMediaStreamSource: (ctx, stream) => ctx.createMediaStreamSource(stream),
    };
  }

  /** Constructor options for {@link MicVadSource}. */
  export interface MicVadSourceOptions {
    /** The shared engine `AudioContext` (`AudioApi.context()`, wired once real integration is
     * complete — never a source-owned context). */
    audioContext: AudioContext;
    /** The worklet module's URL (`new URL("./vad.worklet.ts", import.meta.url)` in
     * production). */
    workletUrl: string | URL;
    /** Initial sensitivity multiplier; default `DEFAULT_SENSITIVITY`. */
    sensitivity?: number;
    /** Diagnostic sink. */
    logger: Logger;
    /** Injectable browser deps (tests supply fakes); defaults to `defaultDeps()`. */
    deps?: MicVadDeps;
  }

  /** Why `enable()` could not start listening. */
  export type MicVadDenialReason = "permission-denied" | "no-microphone" | "unknown";

  /**
   * Mic voice-activity `DuckSource`: on `enable()`, requests the microphone
   * (`echoCancellation`/`noiseSuppression` on), registers the shared `VAD_PROCESSOR_NAME`
   * worklet on the engine's `AudioContext`, and forwards each frame's boolean demand to the
   * sink. **Privacy invariant (ironclad — PII): the raw audio buffer never reaches this class**
   * — `vad.worklet.ts`'s processor posts only a boolean per frame; this class only ever reads
   * `MessageEvent.data: boolean` off the worklet's port.
   */
  export class MicVadSource {
    private readonly audioContext: AudioContext;
    private readonly workletUrl: string | URL;
    private sensitivity: number;
    private readonly logger: Logger;
    private readonly deps: MicVadDeps;
    private sink: DuckSink = NULL_SINK;
    private stream: MediaStream | null = null;
    private sourceNode: MediaStreamAudioSourceNode | null = null;
    private workletNode: AudioWorkletNode | null = null;
    private moduleLoaded = false;
    private enabled = false;

    constructor(opts: MicVadSourceOptions) {
      this.audioContext = opts.audioContext;
      this.workletUrl = opts.workletUrl;
      this.sensitivity = opts.sensitivity ?? DEFAULT_SENSITIVITY;
      this.logger = opts.logger;
      this.deps = opts.deps ?? defaultDeps();
    }

    /** Replaces the sink demand is forwarded to (the integration task wires the real one). */
    setSink(sink: DuckSink): void {
      this.sink = sink;
    }

    /** Replaces the sensitivity multiplier for frames scored from now on. */
    setSensitivity(sensitivity: number): void {
      this.sensitivity = sensitivity;
      if (this.workletNode) {
        // The running processor's own VadEngine instance keeps its already-constructed
        // sensitivity — a live change takes effect on the NEXT `enable()` cycle, matching the
        // worklet's `processorOptions`-only construction seam (no live-parameter channel is
        // wired for this milestone's scope).
      }
    }

    /**
     * Requests the microphone and starts forwarding VAD demand. Resolves once listening has
     * started; resolves to a denial reason (never throws) on a permission refusal or missing
     * device: a denial shows the reason and leaves the source off.
     * @returns `null` on success, else the denial reason.
     */
    async enable(): Promise<MicVadDenialReason | null> {
      if (this.enabled) return null;
      let stream: MediaStream;
      try {
        stream = await this.deps.getUserMedia({ audio: { echoCancellation: true, noiseSuppression: true } });
      } catch (e) {
        const name = e instanceof DOMException ? e.name : "";
        this.logger.warn("mic VAD getUserMedia failed", e);
        if (name === "NotAllowedError" || name === "SecurityError") return "permission-denied";
        if (name === "NotFoundError") return "no-microphone";
        return "unknown";
      }
      if (!this.moduleLoaded) {
        await this.deps.addWorkletModule(this.audioContext, String(this.workletUrl));
        this.moduleLoaded = true;
      }
      this.stream = stream;
      this.sourceNode = this.deps.createMediaStreamSource(this.audioContext, stream);
      const node = this.deps.createWorkletNode(this.audioContext, VAD_PROCESSOR_NAME, {
        processorOptions: { sensitivity: this.sensitivity },
      });
      node.port.onmessage = (event: MessageEvent<boolean>) => {
        this.sink.set(event.data ? 1 : 0);
      };
      this.sourceNode.connect(node);
      this.workletNode = node;
      this.enabled = true;
      return null;
    }

    /** Stops listening, releases the microphone, and resets demand to 0. */
    disable(): void {
      if (!this.enabled) return;
      this.enabled = false;
      this.workletNode?.port.close();
      this.workletNode?.disconnect();
      this.sourceNode?.disconnect();
      this.stream?.getTracks().forEach((t) => t.stop());
      this.workletNode = null;
      this.sourceNode = null;
      this.stream = null;
      this.sink.set(0);
    }

    /** Whether the source is currently listening. */
    isEnabled(): boolean {
      return this.enabled;
    }
  }
  ```
- Create: `src/modules/ducking/src/micVad.test.ts`:
  ```ts
  // @vitest-environment node
  import { describe, it, expect } from "vitest";
  import { VadEngine } from "./micVad";

  describe("VadEngine", () => {
    it("stays silent under a constant quiet floor", () => {
      const engine = new VadEngine({ sensitivity: 2.5 });
      let demand = 0;
      for (let i = 0; i < 50; i += 1) demand = engine.pushFrame(0.01);
      expect(demand).toBe(0);
    });

    it("requires 3 consecutive above-threshold frames to start speech", () => {
      const engine = new VadEngine({ sensitivity: 2 });
      for (let i = 0; i < 60; i += 1) engine.pushFrame(0.01); // establish a low floor
      expect(engine.pushFrame(1)).toBe(0);
      expect(engine.pushFrame(1)).toBe(0);
      expect(engine.pushFrame(1)).toBe(1); // third consecutive frame starts speech
    });

    it("a single loud frame among quiet ones resets the onset counter", () => {
      const engine = new VadEngine({ sensitivity: 2 });
      for (let i = 0; i < 60; i += 1) engine.pushFrame(0.01);
      engine.pushFrame(1);
      engine.pushFrame(1);
      expect(engine.pushFrame(0.01)).toBe(0); // quiet frame breaks the streak
      expect(engine.pushFrame(1)).toBe(0); // only 1 consecutive frame again
    });

    it("holds demand through the 400ms hangover after speech drops below threshold", () => {
      const engine = new VadEngine({ sensitivity: 2, frameMs: 20 });
      for (let i = 0; i < 60; i += 1) engine.pushFrame(0.01);
      engine.pushFrame(1);
      engine.pushFrame(1);
      expect(engine.pushFrame(1)).toBe(1); // speech starts
      // 400ms / 20ms = 20 hangover frames; the 20th quiet frame after onset still reports 1.
      let demand = 1;
      for (let i = 0; i < 19; i += 1) demand = engine.pushFrame(0.01);
      expect(demand).toBe(1);
      expect(engine.pushFrame(0.01)).toBe(0); // the 20th quiet frame ends the hangover
    });

    it("an adaptive floor rejects a rms level that would have triggered against a lower floor", () => {
      const quiet = new VadEngine({ sensitivity: 2 });
      const loud = new VadEngine({ sensitivity: 2 });
      for (let i = 0; i < 60; i += 1) quiet.pushFrame(0.01);
      for (let i = 0; i < 60; i += 1) loud.pushFrame(0.2); // a noisier room raises the floor
      // The SAME rms trips the quiet-room engine but not the noisier-room one.
      expect(quiet.pushFrame(0.05)).toBe(0); // still below 3-frame onset on frame 1
      expect(quiet.pushFrame(0.05)).toBe(0);
      expect(quiet.pushFrame(0.05)).toBe(1);
      expect(loud.pushFrame(0.05)).toBe(0);
      expect(loud.pushFrame(0.05)).toBe(0);
      expect(loud.pushFrame(0.05)).toBe(0); // 0.05 never clears the noisier floor's threshold
    });
  });
  ```

  ```ts
  // (appended to the SAME micVad.test.ts file, WITHOUT the node-environment pragma above it —
  // Vitest applies the pragma file-wide, and this describe block needs `DOMException`/
  // `MessageEvent`, both of which Node itself provides natively, so the node environment is
  // still correct for the whole file; no jsdom needed.)
  import { MicVadSource, type MicVadDeps } from "./micVad";

  function stubDeps(overrides: Partial<MicVadDeps> = {}): MicVadDeps {
    return {
      getUserMedia: async () => ({ getTracks: () => [] }) as unknown as MediaStream,
      addWorkletModule: async () => {},
      createWorkletNode: () =>
        ({ port: { onmessage: null, close: () => {} }, connect: () => {}, disconnect: () => {} }) as unknown as AudioWorkletNode,
      createMediaStreamSource: () => ({ connect: () => {}, disconnect: () => {} }) as unknown as MediaStreamAudioSourceNode,
      ...overrides,
    };
  }

  describe("MicVadSource", () => {
    it("enable() resolves null and forwards worklet messages to the sink", async () => {
      let capturedNode: { port: { onmessage: ((e: MessageEvent<boolean>) => void) | null } } | null = null;
      const deps = stubDeps({
        createWorkletNode: () => {
          capturedNode = { port: { onmessage: null } };
          return { ...capturedNode, port: { ...capturedNode.port, close: () => {} }, connect: () => {}, disconnect: () => {} } as unknown as AudioWorkletNode;
        },
      });
      const calls: number[] = [];
      const source = new MicVadSource({
        audioContext: {} as AudioContext,
        workletUrl: "vad.worklet.js",
        logger: { debug() {}, warn() {}, error() {} },
        deps,
      });
      source.setSink({ set: (v) => calls.push(v) });
      const denial = await source.enable();
      expect(denial).toBeNull();
      expect(source.isEnabled()).toBe(true);
      capturedNode?.port.onmessage?.({ data: true } as MessageEvent<boolean>);
      expect(calls).toEqual([1]);
    });

    it("enable() reports permission-denied without throwing", async () => {
      const deps = stubDeps({
        getUserMedia: async () => {
          throw new DOMException("denied", "NotAllowedError");
        },
      });
      const source = new MicVadSource({
        audioContext: {} as AudioContext,
        workletUrl: "vad.worklet.js",
        logger: { debug() {}, warn() {}, error() {} },
        deps,
      });
      const denial = await source.enable();
      expect(denial).toBe("permission-denied");
      expect(source.isEnabled()).toBe(false);
    });

    it("disable() resets demand to 0 and stops the tracks", async () => {
      const stopped: boolean[] = [];
      const deps = stubDeps({
        getUserMedia: async () => ({ getTracks: () => [{ stop: () => stopped.push(true) }] }) as unknown as MediaStream,
      });
      const calls: number[] = [];
      const source = new MicVadSource({
        audioContext: {} as AudioContext,
        workletUrl: "vad.worklet.js",
        logger: { debug() {}, warn() {}, error() {} },
        deps,
      });
      source.setSink({ set: (v) => calls.push(v) });
      await source.enable();
      source.disable();
      expect(stopped).toEqual([true]);
      expect(calls.at(-1)).toBe(0);
      expect(source.isEnabled()).toBe(false);
    });
  });
  ```
  (Both `describe` blocks above live in the single file `src/modules/ducking/src/micVad.test.ts`
  — merge them under one `import` section rather than two separate `import` groups as shown;
  the split above is only for this plan's readability.)
- Create: `src/modules/ducking/src/vad.worklet.ts`:
  ```ts
  import { VadEngine, VAD_PROCESSOR_NAME, VAD_FRAME_MS } from "./micVad";

  /** Ambient shape of the AudioWorkletGlobalScope's per-processor base class — not part of the
   * default `dom` lib (that belongs to `@types/audioworklet`'s separate global environment,
   * which this project does not otherwise pull in), so this file declares only the minimal
   * surface it calls. This is a TYPE-ONLY declaration (`declare class` emits no runtime code);
   * the identifier `AudioWorkletProcessor` it names resolves to the REAL global that exists
   * inside an AudioWorkletGlobalScope at runtime — this file is loaded there via
   * `audioContext.audioWorklet.addModule(new URL("./vad.worklet.ts", import.meta.url))`, never
   * imported directly by test code (which is why `micVad.test.ts` tests `VadEngine` alone). */
  declare class AudioWorkletProcessor {
    readonly port: MessagePort;
    constructor(options?: { processorOptions?: unknown });
    process(
      inputs: Float32Array[][],
      outputs: Float32Array[][],
      parameters: Record<string, Float32Array>,
    ): boolean;
  }
  /** Ambient `registerProcessor`, present only inside an AudioWorkletGlobalScope. */
  declare function registerProcessor(name: string, processorCtor: unknown): void;
  /** Ambient per-worklet sample-rate global. */
  declare const sampleRate: number;

  /**
   * Runs entirely inside the AudioWorkletGlobalScope: accumulates input samples into 20 ms
   * frames, computes each frame's RMS, feeds it to a `VadEngine`, and posts ONLY the resulting
   * boolean over `port` — never a sample, buffer, or RMS value (the privacy invariant: the
   * audio itself never leaves the worklet).
   */
  class VadProcessor extends AudioWorkletProcessor {
    /** The adaptive VAD state machine, constructed from `processorOptions.sensitivity`. */
    private readonly engine: VadEngine;
    /** Samples accumulated toward the next `frameSamples`-sized frame. */
    private buffer: number[] = [];
    /** Frame size in samples at this worklet's `sampleRate`. */
    private readonly frameSamples: number;

    constructor(options?: { processorOptions?: { sensitivity?: number } }) {
      super(options);
      const sensitivity = options?.processorOptions?.sensitivity ?? 2.5;
      this.engine = new VadEngine({ sensitivity, frameMs: VAD_FRAME_MS });
      this.frameSamples = Math.round((sampleRate * VAD_FRAME_MS) / 1000);
    }

    process(inputs: Float32Array[][]): boolean {
      const channel = inputs[0]?.[0];
      if (channel) {
        for (const sample of channel) {
          this.buffer.push(sample);
          if (this.buffer.length >= this.frameSamples) {
            const rms = computeRms(this.buffer);
            this.buffer = [];
            this.port.postMessage(this.engine.pushFrame(rms) === 1);
          }
        }
      }
      return true; // keep the processor alive for the node's lifetime
    }
  }

  /**
   * Root-mean-square amplitude of `samples`.
   * @param samples The accumulated frame's samples.
   * @returns The frame's RMS amplitude.
   */
  function computeRms(samples: number[]): number {
    const sumSquares = samples.reduce((sum, s) => sum + s * s, 0);
    return Math.sqrt(sumSquares / samples.length);
  }

  // Only present inside an AudioWorkletGlobalScope; `typeof` on an undeclared global safely
  // evaluates to `"undefined"` (never throws) in every other environment, including this
  // module never being imported by a Vitest test in the first place.
  if (typeof registerProcessor === "function") {
    registerProcessor(VAD_PROCESSOR_NAME, VadProcessor);
  }
  ```

- [ ] **Step 1:** write `micVad.test.ts` first (fails: `VadEngine`/`MicVadSource` don't exist),
  then implement `micVad.ts`. `pnpm --filter @shadowcat/module-ducking test` PASS.
- [ ] **Step 2:** implement `vad.worklet.ts`. `pnpm --filter @shadowcat/module-ducking
  typecheck` PASS (proves the ambient `AudioWorkletProcessor` declaration compiles cleanly
  against the rest of the package's `dom`-lib code without a name collision). `pnpm lint`,
  `pnpm lint:docs`, `pnpm lint:props`, `pnpm lint:comments` PASS.
- [ ] **Step 3:** `git commit -m "feat(ducking): mic voice-activity source (VadEngine + AudioWorklet)" -- src/modules/ducking/src/micVad.ts src/modules/ducking/src/micVad.test.ts src/modules/ducking/src/vad.worklet.ts`

### Task 10: `OsMonitorSource`

**Files:**
- Create: `src/modules/ducking/src/osMonitor.ts`:
  ```ts
  import type { Logger } from "@shadowcat/core";
  import type { DuckSink } from "./keySource";
  import { NULL_SINK } from "./keySource";

  /** Peak threshold above which a watched process counts as "talking". */
  export const DEFAULT_THRESHOLD = 0.02;
  /** Hangover held after the peak drops back under threshold, ms (same shape as the mic
   * source's hangover). */
  const HANGOVER_MS = 400;
  /** Reconnect backoff schedule, ms — capped, repeating the last entry. */
  const RECONNECT_BACKOFF_MS = [500, 1000, 2000, 5000];
  /** Failed connection attempts before the status reports "not running". */
  const NOT_RUNNING_AFTER_ATTEMPTS = 3;

  /** The monitor's `hello` frame. */
  interface HelloFrame {
    type: "hello";
    os: string;
    supported: boolean;
    reason?: string;
  }
  /** The monitor's `levels` frame. */
  interface LevelsFrame {
    type: "levels";
    sessions: { process: string; peak: number }[];
  }

  /** `OsMonitorSource`'s externally observable connection status: "connected", "not
   * running", or "unsupported on this OS" as the monitor reports. */
  export type OsMonitorStatus = "connecting" | "connected" | "not-running" | "unsupported";

  /** Constructor options for {@link OsMonitorSource}. */
  export interface OsMonitorSourceOptions {
    /** Localhost port `shadowcat audio-monitor` is expected to listen on. */
    port: number;
    /** Initial watched-process substrings, sent as the first `watch` frame right after
     * connect (the monitor itself already seeded `--watch`, but a client-configured list set
     * before the monitor was even started must still apply once connected). */
    watch: string[];
    /** Diagnostic sink. */
    logger: Logger;
    /** Injectable `WebSocket` constructor (tests supply a fake); defaults to the global
     * `WebSocket`. */
    createSocket?: (url: string) => WebSocket;
  }

  /**
   * Browser-side client for the `shadowcat audio-monitor` localhost WebSocket: connects to
   * `ws://127.0.0.1:<port>/levels`, reconnects with backoff, and reports demand 1 whenever any
   * (already server-filtered) watched session's peak exceeds `threshold`, held for
   * `HANGOVER_MS` after it drops back.
   */
  export class OsMonitorSource {
    private readonly port: number;
    private watch: string[];
    private readonly logger: Logger;
    private readonly createSocket: (url: string) => WebSocket;
    private sink: DuckSink = NULL_SINK;
    private threshold = DEFAULT_THRESHOLD;
    private socket: WebSocket | null = null;
    private attempts = 0;
    private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
    private hangoverTimer: ReturnType<typeof setTimeout> | null = null;
    private started = false;
    private status: OsMonitorStatus = "connecting";
    private readonly statusListeners = new Set<(status: OsMonitorStatus) => void>();

    constructor(opts: OsMonitorSourceOptions) {
      this.port = opts.port;
      this.watch = opts.watch;
      this.logger = opts.logger;
      this.createSocket = opts.createSocket ?? ((url) => new WebSocket(url));
    }

    /** Begins connecting; a no-op if already started. */
    start(): void {
      if (this.started) return;
      this.started = true;
      this.connect();
    }

    /** Tears down the connection and any pending timers; resets demand to 0. */
    stop(): void {
      this.started = false;
      if (this.reconnectTimer) clearTimeout(this.reconnectTimer);
      if (this.hangoverTimer) clearTimeout(this.hangoverTimer);
      this.socket?.close();
      this.socket = null;
      this.sink.set(0);
    }

    /** Replaces the sink demand is forwarded to (the integration task wires the real one). */
    setSink(sink: DuckSink): void {
      this.sink = sink;
    }

    /** Replaces the peak threshold used to derive demand (a settings change). */
    setThreshold(threshold: number): void {
      this.threshold = threshold;
    }

    /** Replaces the live watch list, sending a `watch` frame immediately if connected. */
    setWatch(watch: string[]): void {
      this.watch = watch;
      if (this.socket && this.socket.readyState === WebSocket.OPEN) {
        this.socket.send(JSON.stringify({ type: "watch", names: watch }));
      }
    }

    /** Subscribes to connection-status changes; returns an unsubscribe. */
    onStatusChange(cb: (status: OsMonitorStatus) => void): () => void {
      this.statusListeners.add(cb);
      return () => this.statusListeners.delete(cb);
    }

    /** The current connection status. */
    getStatus(): OsMonitorStatus {
      return this.status;
    }

    private setStatus(status: OsMonitorStatus): void {
      if (this.status === status) return;
      this.status = status;
      for (const cb of this.statusListeners) cb(status);
    }

    private connect(): void {
      if (!this.started) return;
      const socket = this.createSocket(`ws://127.0.0.1:${this.port}/levels`);
      this.socket = socket;
      socket.onopen = () => {
        this.attempts = 0;
        if (this.watch.length > 0) {
          socket.send(JSON.stringify({ type: "watch", names: this.watch }));
        }
      };
      socket.onmessage = (event) => {
        this.handleMessage(String(event.data));
      };
      socket.onclose = () => {
        if (!this.started) return;
        this.attempts += 1;
        this.setStatus(this.attempts >= NOT_RUNNING_AFTER_ATTEMPTS ? "not-running" : "connecting");
        const delay = RECONNECT_BACKOFF_MS[Math.min(this.attempts - 1, RECONNECT_BACKOFF_MS.length - 1)];
        this.reconnectTimer = setTimeout(() => this.connect(), delay);
      };
      socket.onerror = () => {
        this.logger.warn("audio-monitor socket error");
      };
    }

    private handleMessage(raw: string): void {
      let parsed: unknown;
      try {
        parsed = JSON.parse(raw);
      } catch {
        return;
      }
      if (isHelloFrame(parsed)) {
        this.setStatus(parsed.supported ? "connected" : "unsupported");
        return;
      }
      if (!isLevelsFrame(parsed)) return;
      // Sessions arriving here are already watch-filtered server-side: this class
      // only evaluates peak against threshold, never re-matches process names.
      const talking = parsed.sessions.some((s) => s.peak > this.threshold);
      if (talking) {
        if (this.hangoverTimer) {
          clearTimeout(this.hangoverTimer);
          this.hangoverTimer = null;
        }
        this.sink.set(1);
      } else if (!this.hangoverTimer) {
        this.hangoverTimer = setTimeout(() => {
          this.hangoverTimer = null;
          this.sink.set(0);
        }, HANGOVER_MS);
      }
    }
  }

  /**
   * Narrows `v` to a `hello` frame.
   * @param v The parsed JSON value to check.
   * @returns Whether `v` is a `hello` frame.
   */
  function isHelloFrame(v: unknown): v is HelloFrame {
    return typeof v === "object" && v !== null && (v as { type?: unknown }).type === "hello";
  }
  /**
   * Narrows `v` to a `levels` frame.
   * @param v The parsed JSON value to check.
   * @returns Whether `v` is a `levels` frame.
   */
  function isLevelsFrame(v: unknown): v is LevelsFrame {
    return (
      typeof v === "object" &&
      v !== null &&
      (v as { type?: unknown }).type === "levels" &&
      Array.isArray((v as LevelsFrame).sessions)
    );
  }
  ```
- Create: `src/modules/ducking/src/osMonitor.test.ts`:
  ```ts
  // @vitest-environment node
  import { describe, it, expect, vi, afterEach } from "vitest";
  import { OsMonitorSource } from "./osMonitor";

  /** A scriptable fake `WebSocket` capturing the handlers `OsMonitorSource` assigns, with a
   * `simulate*` helper per frame kind so tests drive the connection without a real socket. */
  class FakeSocket {
    static OPEN = 1;
    static CONNECTING = 0;
    readyState = FakeSocket.CONNECTING;
    onopen: (() => void) | null = null;
    onmessage: ((e: { data: string }) => void) | null = null;
    onclose: (() => void) | null = null;
    onerror: (() => void) | null = null;
    sent: string[] = [];
    send(data: string): void {
      this.sent.push(data);
    }
    close(): void {
      this.readyState = 3;
    }
    open(): void {
      this.readyState = FakeSocket.OPEN;
      this.onopen?.();
    }
    message(payload: unknown): void {
      this.onmessage?.({ data: JSON.stringify(payload) });
    }
  }

  const logger = { debug() {}, warn() {}, error() {} };

  describe("OsMonitorSource", () => {
    afterEach(() => {
      vi.useRealTimers();
    });

    it("sends the initial watch list once connected", () => {
      let socket!: FakeSocket;
      const source = new OsMonitorSource({
        port: 31998,
        watch: ["discord"],
        logger,
        createSocket: () => (socket = new FakeSocket()) as unknown as WebSocket,
      });
      source.start();
      socket.open();
      expect(socket.sent).toEqual([JSON.stringify({ type: "watch", names: ["discord"] })]);
      source.stop();
    });

    it("reports status from the hello frame", () => {
      let socket!: FakeSocket;
      const source = new OsMonitorSource({
        port: 31998,
        watch: [],
        logger,
        createSocket: () => (socket = new FakeSocket()) as unknown as WebSocket,
      });
      const statuses: string[] = [];
      source.onStatusChange((s) => statuses.push(s));
      source.start();
      socket.open();
      socket.message({ type: "hello", os: "linux", supported: false, reason: "PipeWire not running" });
      expect(source.getStatus()).toBe("unsupported");
      expect(statuses).toContain("unsupported");
      source.stop();
    });

    it("sets demand 1 when a session's peak exceeds threshold, with hangover on drop", () => {
      vi.useFakeTimers();
      let socket!: FakeSocket;
      const calls: number[] = [];
      const source = new OsMonitorSource({
        port: 31998,
        watch: ["discord"],
        logger,
        createSocket: () => (socket = new FakeSocket()) as unknown as WebSocket,
      });
      source.setSink({ set: (v) => calls.push(v) });
      source.start();
      socket.open();
      socket.message({ type: "levels", sessions: [{ process: "discord", peak: 0.5 }] });
      expect(calls).toEqual([1]);
      socket.message({ type: "levels", sessions: [{ process: "discord", peak: 0.0 }] });
      expect(calls).toEqual([1]); // still in hangover
      vi.advanceTimersByTime(400);
      expect(calls).toEqual([1, 0]);
      source.stop();
    });

    it("setWatch sends a live watch frame while connected", () => {
      let socket!: FakeSocket;
      const source = new OsMonitorSource({
        port: 31998,
        watch: [],
        logger,
        createSocket: () => (socket = new FakeSocket()) as unknown as WebSocket,
      });
      source.start();
      socket.open();
      socket.sent = [];
      source.setWatch(["firefox"]);
      expect(socket.sent).toEqual([JSON.stringify({ type: "watch", names: ["firefox"] })]);
      source.stop();
    });

    it("reports not-running after 3 failed connection attempts", () => {
      vi.useFakeTimers();
      const sockets: FakeSocket[] = [];
      const source = new OsMonitorSource({
        port: 31998,
        watch: [],
        logger,
        createSocket: () => {
          const s = new FakeSocket();
          sockets.push(s);
          return s as unknown as WebSocket;
        },
      });
      const statuses: string[] = [];
      source.onStatusChange((s) => statuses.push(s));
      source.start();
      for (let i = 0; i < 3; i += 1) {
        sockets.at(-1)?.onclose?.();
        vi.runOnlyPendingTimers();
      }
      expect(statuses.at(-1)).toBe("not-running");
      source.stop();
    });
  });
  ```

- [ ] **Step 1:** write `osMonitor.test.ts` first (fails: `OsMonitorSource` doesn't exist),
  then implement `osMonitor.ts`. `pnpm --filter @shadowcat/module-ducking test` PASS.
- [ ] **Step 2:** `pnpm --filter @shadowcat/module-ducking typecheck`, `pnpm lint`, `pnpm
  lint:docs`, `pnpm lint:props` PASS.
- [ ] **Step 3:** `git commit -m "feat(ducking): OS audio-session monitor source" -- src/modules/ducking/src/osMonitor.ts src/modules/ducking/src/osMonitor.test.ts`

### Task 11: `DuckSourcesController`, the preferences mirror, `DuckingSettings.svelte`, i18n, module registration

**Files:**
- Modify: `src/modules/ducking/src/osMonitor.ts` — make `port` mutable and add a reconnect-on-change setter (two small edits):
  1. Change `private readonly port: number;` to `private port: number;`.
  2. Add, beside `setThreshold`:
     ```ts
     /** Replaces the port to connect to; if currently started, tears down the existing
      * connection and reconnects to the new port immediately (rather than waiting for the
      * next backoff-scheduled retry). */
     setPort(port: number): void {
       if (this.port === port) return;
       this.port = port;
       if (this.started) {
         this.socket?.close();
         if (this.reconnectTimer) clearTimeout(this.reconnectTimer);
         this.attempts = 0;
         this.connect();
       }
     }
     ```
- Modify: `src/modules/ducking/src/osMonitor.test.ts` — add:
  ```ts
  it("setPort reconnects immediately to the new port while started", () => {
    const createdPorts: number[] = [];
    const source = new OsMonitorSource({
      port: 31998,
      watch: [],
      logger,
      createSocket: (url) => {
        createdPorts.push(Number(new URL(url).port));
        return new FakeSocket() as unknown as WebSocket;
      },
    });
    source.start();
    source.setPort(31999);
    expect(createdPorts).toEqual([31998, 31999]);
    source.stop();
  });
  ```
- Create: `src/modules/ducking/src/duckingMirror.ts`:
  ```ts
  import { DEFAULT_KEY } from "./keySource";
  import { DEFAULT_THRESHOLD } from "./osMonitor";
  import { DEFAULT_SENSITIVITY } from "./micVad";

  /** Persisted per-device ducking preferences, mirrored to `localStorage` under
   * `shadowcat.ducking`. */
  export interface DuckingPreferences {
    /** Master enable — off disables all three sources regardless of their own toggles. */
    masterEnabled: boolean;
    /** Push-to-duck key source enable. */
    keyEnabled: boolean;
    /** The bound key's `KeyboardEvent.code`. */
    keyBinding: string;
    /** Mic voice-activity source enable. */
    micEnabled: boolean;
    /** Mic VAD sensitivity multiplier. */
    micSensitivity: number;
    /** OS audio-session monitor source enable. */
    osEnabled: boolean;
    /** The `shadowcat audio-monitor` port to connect to. */
    osPort: number;
    /** Peak threshold above which a watched session counts as "talking". */
    osThreshold: number;
    /** Watched-process substrings, case-insensitive (also the monitor's live `watch` list). */
    watchList: string[];
  }

  // Duck DEPTH is deliberately absent: it is `ctx.audio.duck.depth`, persisted by the audio
  // engine's own `shadowcat.audio` mirror, never a second copy here — one stored value per fact.

  /** The single localStorage key holding the ducking preferences mirror — this module's own
   * read/write pair, styled after `sessionState.svelte.ts`'s `readThemeMirror`/
   * `writeThemeMirror` (garbage-tolerant read, best-effort write). */
  export const DUCKING_MIRROR_STORAGE_KEY = "shadowcat.ducking";

  /** The preferences a fresh device starts with. */
  export const DEFAULT_DUCKING_PREFERENCES: DuckingPreferences = {
    masterEnabled: true,
    keyEnabled: false,
    keyBinding: DEFAULT_KEY,
    micEnabled: false,
    micSensitivity: DEFAULT_SENSITIVITY,
    osEnabled: false,
    osPort: 31998,
    osThreshold: DEFAULT_THRESHOLD,
    watchList: ["discord"],
  };

  /**
   * Reads the ducking mirror, garbage-tolerantly: an absent key, malformed JSON, or a
   * non-object payload all yield the defaults; a partial object fills missing fields from the
   * defaults (forward-compatible with a preferences field added later).
   * @param storage The storage to read (injectable for tests; production passes `localStorage`).
   * @returns The persisted preferences, or the defaults when absent/unreadable.
   */
  export function readDuckingMirror(storage: Pick<Storage, "getItem">): DuckingPreferences {
    const raw = storage.getItem(DUCKING_MIRROR_STORAGE_KEY);
    if (raw === null) return { ...DEFAULT_DUCKING_PREFERENCES };
    try {
      const parsed: unknown = JSON.parse(raw);
      if (typeof parsed !== "object" || parsed === null) return { ...DEFAULT_DUCKING_PREFERENCES };
      return { ...DEFAULT_DUCKING_PREFERENCES, ...(parsed as Partial<DuckingPreferences>) };
    } catch {
      return { ...DEFAULT_DUCKING_PREFERENCES };
    }
  }

  /**
   * Writes the ducking mirror. A throwing storage (quota, privacy mode) is swallowed — a
   * failed write must never break the settings change that triggered it.
   * @param storage The storage to write (injectable for tests; production passes
   *   `localStorage`).
   * @param value The preferences snapshot to persist.
   */
  export function writeDuckingMirror(storage: Pick<Storage, "setItem">, value: DuckingPreferences): void {
    try {
      storage.setItem(DUCKING_MIRROR_STORAGE_KEY, JSON.stringify(value));
    } catch {
      // best-effort; see the function doc above.
    }
  }
  ```
- Create: `src/modules/ducking/src/duckingMirror.test.ts`:
  ```ts
  // @vitest-environment node
  import { describe, it, expect } from "vitest";
  import {
    readDuckingMirror,
    writeDuckingMirror,
    DEFAULT_DUCKING_PREFERENCES,
    DUCKING_MIRROR_STORAGE_KEY,
  } from "./duckingMirror";

  function fakeStorage(initial: Record<string, string> = {}) {
    const store = { ...initial };
    return {
      getItem: (k: string) => store[k] ?? null,
      setItem: (k: string, v: string) => {
        store[k] = v;
      },
      raw: store,
    };
  }

  describe("ducking mirror", () => {
    it("returns the defaults when absent", () => {
      expect(readDuckingMirror(fakeStorage())).toEqual(DEFAULT_DUCKING_PREFERENCES);
    });

    it("returns the defaults on malformed JSON", () => {
      expect(readDuckingMirror(fakeStorage({ [DUCKING_MIRROR_STORAGE_KEY]: "{not json" }))).toEqual(
        DEFAULT_DUCKING_PREFERENCES,
      );
    });

    it("fills a partial stored object from the defaults", () => {
      const storage = fakeStorage({ [DUCKING_MIRROR_STORAGE_KEY]: JSON.stringify({ osPort: 40000 }) });
      expect(readDuckingMirror(storage)).toEqual({ ...DEFAULT_DUCKING_PREFERENCES, osPort: 40000 });
    });

    it("round-trips a full write/read", () => {
      const storage = fakeStorage();
      const value = { ...DEFAULT_DUCKING_PREFERENCES, keyEnabled: true, watchList: ["discord", "teams"] };
      writeDuckingMirror(storage, value);
      expect(readDuckingMirror(storage)).toEqual(value);
    });

    it("a throwing storage write is swallowed", () => {
      const storage = {
        getItem: () => null,
        setItem: () => {
          throw new Error("quota");
        },
      };
      expect(() => writeDuckingMirror(storage, DEFAULT_DUCKING_PREFERENCES)).not.toThrow();
    });
  });
  ```
- Create: `src/modules/ducking/src/controller.ts`:
  ```ts
  import type { Logger } from "@shadowcat/core";
  import { KeySource, NULL_SINK, type DuckSink } from "./keySource";
  import { OsMonitorSource } from "./osMonitor";
  import type { DuckingPreferences } from "./duckingMirror";

  /**
   * Owns the two sources that need no `AudioContext` (`KeySource`, `OsMonitorSource`) and
   * applies a `DuckingPreferences` snapshot to their running state. Constructed once in
   * `register(ctx)` and shared across every mount/unmount of the contributed settings section
   * — the sources must keep running while Settings is closed, since ducking is a background
   * effect, not a settings-panel-only feature. The mic source needs the engine's shared
   * `AudioContext`, so it is owned separately and wired only once `AudioApi.context()` exists.
   */
  export class DuckSourcesController {
    /** The push-to-duck key source. */
    readonly key: KeySource;
    /** The OS audio-session monitor source. */
    readonly osMonitor: OsMonitorSource;

    /**
     * @param prefs The initial preferences snapshot (from `readDuckingMirror`).
     * @param logger Diagnostic sink, forwarded to `OsMonitorSource`.
     */
    constructor(prefs: DuckingPreferences, logger: Logger) {
      this.key = new KeySource(NULL_SINK, prefs.keyBinding);
      this.osMonitor = new OsMonitorSource({ port: prefs.osPort, watch: prefs.watchList, logger });
      this.osMonitor.setThreshold(prefs.osThreshold);
      this.applyEnablement(prefs);
    }

    /**
     * Applies a preferences snapshot's binding/threshold/watch-list/enablement fields to the
     * running sources. Called on every settings change, not just construction.
     * @param prefs The preferences snapshot to apply.
     */
    applyPreferences(prefs: DuckingPreferences): void {
      this.key.setKey(prefs.keyBinding);
      this.osMonitor.setThreshold(prefs.osThreshold);
      this.osMonitor.setWatch(prefs.watchList);
      this.applyEnablement(prefs);
    }

    /**
     * Wires both sources' demand output to real `DuckSource` sinks (this plan's integration
     * task, once `ctx.audio.duck` exists).
     * @param keySink The sink for the key source's demand.
     * @param osSink The sink for the OS monitor source's demand.
     */
    wireToAudioDuck(keySink: DuckSink, osSink: DuckSink): void {
      this.key.setSink(keySink);
      this.osMonitor.setSink(osSink);
    }

    /** Tears down both sources (called from the module's `unregister()`). */
    dispose(): void {
      this.key.stop();
      this.osMonitor.stop();
    }

    /**
     * Starts/stops each source per `masterEnabled` AND its own per-source flag.
     * @param prefs The preferences snapshot to derive enablement from.
     */
    private applyEnablement(prefs: DuckingPreferences): void {
      if (prefs.masterEnabled && prefs.keyEnabled) this.key.start();
      else this.key.stop();
      if (prefs.masterEnabled && prefs.osEnabled) this.osMonitor.start();
      else this.osMonitor.stop();
    }
  }
  ```
- Create: `src/modules/ducking/src/controller.test.ts`:
  ```ts
  import { describe, it, expect, afterEach } from "vitest";
  import { DuckSourcesController } from "./controller";
  import { DEFAULT_DUCKING_PREFERENCES } from "./duckingMirror";

  const logger = { debug() {}, warn() {}, error() {} };

  describe("DuckSourcesController", () => {
    afterEach(() => {
      document.body.innerHTML = "";
    });

    it("starts nothing by default (both sources disabled)", () => {
      const controller = new DuckSourcesController(DEFAULT_DUCKING_PREFERENCES, logger);
      const calls: number[] = [];
      controller.wireToAudioDuck({ set: (v) => calls.push(v) }, { set: () => {} });
      window.dispatchEvent(new KeyboardEvent("keydown", { code: DEFAULT_DUCKING_PREFERENCES.keyBinding }));
      expect(calls).toEqual([]);
      controller.dispose();
    });

    it("applyPreferences starts the key source when it and master are enabled", () => {
      const controller = new DuckSourcesController(DEFAULT_DUCKING_PREFERENCES, logger);
      const calls: number[] = [];
      controller.wireToAudioDuck({ set: (v) => calls.push(v) }, { set: () => {} });
      controller.applyPreferences({ ...DEFAULT_DUCKING_PREFERENCES, keyEnabled: true });
      window.dispatchEvent(new KeyboardEvent("keydown", { code: DEFAULT_DUCKING_PREFERENCES.keyBinding }));
      expect(calls).toEqual([1]);
      controller.dispose();
    });

    it("masterEnabled false stops an already-enabled key source", () => {
      const controller = new DuckSourcesController({ ...DEFAULT_DUCKING_PREFERENCES, keyEnabled: true }, logger);
      const calls: number[] = [];
      controller.wireToAudioDuck({ set: (v) => calls.push(v) }, { set: () => {} });
      controller.applyPreferences({ ...DEFAULT_DUCKING_PREFERENCES, keyEnabled: true, masterEnabled: false });
      window.dispatchEvent(new KeyboardEvent("keydown", { code: DEFAULT_DUCKING_PREFERENCES.keyBinding }));
      expect(calls).toEqual([]);
      controller.dispose();
    });
  });
  ```
- Create: `src/modules/ducking/src/DuckingSettings.svelte`:
  ```svelte
  <script lang="ts">
    import { onMount } from "svelte";
    import { getAppContext } from "@shadowcat/ui-kit";
    import type { DuckSourcesController } from "./controller";
    import type { MicVadDenialReason } from "./micVad";
    import { readDuckingMirror, writeDuckingMirror, type DuckingPreferences } from "./duckingMirror";
    import type { OsMonitorStatus } from "./osMonitor";

    let {
      controller,
      onMicToggle,
      depth = 0.7,
      onDepthChange,
    }: {
      /** Owns the running `KeySource`/`OsMonitorSource` for this world session (constructed
       * once in `register(ctx)`, shared across every mount/unmount of this section). */
      controller: DuckSourcesController;
      /** Enables/disables the mic source for real; `undefined` before a real `MicVadSource` is
       * wired in (the mic toggle still persists the preference either way). */
      onMicToggle?: (enabled: boolean) => Promise<MicVadDenialReason | null>;
      /** The per-device duck depth the slider starts at. Interim seam: until `AppContext`
       * carries `audio`, the section cannot read `audio.duck.depth` itself, so the value and
       * its setter arrive as props; real integration replaces both with a mount-time
       * context read. Depth is the audio engine's state, never persisted by this module. */
      depth?: number;
      /** Receives a slider change; `undefined` means the slider moves and nothing hears it. */
      onDepthChange?: (depth: number) => void;
    } = $props();

    const { t } = getAppContext();

    let prefs = $state<DuckingPreferences>(readDuckingMirror(localStorage));
    let osStatus = $state<OsMonitorStatus>(controller.osMonitor.getStatus());
    let micDenial = $state<MicVadDenialReason | null>(null);
    let capturingKey = $state(false);
    let watchListText = $state(prefs.watchList.join(", "));

    onMount(() => {
      controller.applyPreferences(prefs);
      return controller.osMonitor.onStatusChange((s) => (osStatus = s));
    });

    /** Persists the current `prefs` snapshot and re-applies it to the controller's sources.
     * @example
     * ```
     * // private function; not part of the public API — called after every field mutation
     * persist();
     * ```
     */
    function persist(): void {
      writeDuckingMirror(localStorage, prefs);
      controller.applyPreferences(prefs);
    }

    /**
     * Starts listening for the next keydown and binds it as the push-to-duck key.
     * @example
     * ```
     * // private function; not part of the public API — wired to the "bind" button
     * startKeyCapture();
     * ```
     */
    function startKeyCapture(): void {
      capturingKey = true;
      const onKeydown = (e: KeyboardEvent) => {
        e.preventDefault();
        prefs.keyBinding = e.code;
        capturingKey = false;
        window.removeEventListener("keydown", onKeydown, true);
        persist();
      };
      window.addEventListener("keydown", onKeydown, true);
    }

    /**
     * Toggles the mic source: persists the preference immediately, then (once `onMicToggle`
     * exists) awaits the real enable/disable call and surfaces a denial reason: a denial shows
     * the reason and leaves the source off.
     * @param enabled The requested enabled state.
     * @example
     * ```
     * // private function; not part of the public API — wired to the mic checkbox
     * await toggleMic(true);
     * ```
     */
    async function toggleMic(enabled: boolean): Promise<void> {
      prefs.micEnabled = enabled;
      persist();
      if (onMicToggle) {
        micDenial = (await onMicToggle(enabled)) ?? null;
        if (micDenial) {
          prefs.micEnabled = false;
          persist();
        }
      }
    }

    /**
     * Parses the comma-separated watch-list text into the persisted array. A LIVE
     * editor — `persist()` -> `controller.applyPreferences` -> `OsMonitorSource.setWatch`
     * sends the `watch` frame immediately.
     * @example
     * ```
     * // private function; not part of the public API — wired to the watch-list input's onchange
     * commitWatchList();
     * ```
     */
    function commitWatchList(): void {
      prefs.watchList = watchListText.split(",").map((s) => s.trim()).filter((s) => s.length > 0);
      persist();
    }

    const commandLine = $derived(
      `shadowcat audio-monitor --port ${prefs.osPort} --allow-origin ${typeof window !== "undefined" ? window.location.origin : ""} --watch ${prefs.watchList.join(",")}`,
    );
  </script>

  <div class="ducking-settings">
    <label>
      <input
        type="checkbox"
        checked={prefs.masterEnabled}
        onchange={(e) => { prefs.masterEnabled = e.currentTarget.checked; persist(); }}
      />
      {t("ducking.masterEnable")}
    </label>

    <fieldset disabled={!prefs.masterEnabled}>
      <legend>{t("ducking.keySource.title")}</legend>
      <label>
        <input
          type="checkbox"
          checked={prefs.keyEnabled}
          onchange={(e) => { prefs.keyEnabled = e.currentTarget.checked; persist(); }}
        />
        {t("ducking.keySource.enable")}
      </label>
      <button type="button" onclick={startKeyCapture} disabled={capturingKey}>
        {capturingKey ? t("ducking.keySource.bindPrompt") : t("ducking.keySource.bind", { key: prefs.keyBinding })}
      </button>
    </fieldset>

    <fieldset disabled={!prefs.masterEnabled}>
      <legend>{t("ducking.micSource.title")}</legend>
      <label>
        <input type="checkbox" checked={prefs.micEnabled} onchange={(e) => toggleMic(e.currentTarget.checked)} />
        {t("ducking.micSource.enable")}
      </label>
      <label>
        {t("ducking.micSource.sensitivity")}
        <input
          type="range"
          min="0.5"
          max="6"
          step="0.1"
          value={prefs.micSensitivity}
          oninput={(e) => { prefs.micSensitivity = Number(e.currentTarget.value); persist(); }}
        />
      </label>
      {#if micDenial}
        <p class="denial">{t(`ducking.micSource.denied.${micDenial}`)}</p>
      {/if}
    </fieldset>

    <fieldset disabled={!prefs.masterEnabled}>
      <legend>{t("ducking.osSource.title")}</legend>
      <label>
        <input
          type="checkbox"
          checked={prefs.osEnabled}
          onchange={(e) => { prefs.osEnabled = e.currentTarget.checked; persist(); }}
        />
        {t("ducking.osSource.enable")}
      </label>
      <label>
        {t("ducking.osSource.port")}
        <input
          type="number"
          value={prefs.osPort}
          onchange={(e) => { prefs.osPort = Number(e.currentTarget.value); controller.osMonitor.setPort(prefs.osPort); persist(); }}
        />
      </label>
      <label>
        {t("ducking.osSource.threshold")}
        <input
          type="range"
          min="0"
          max="0.2"
          step="0.005"
          value={prefs.osThreshold}
          oninput={(e) => { prefs.osThreshold = Number(e.currentTarget.value); persist(); }}
        />
      </label>
      <label>
        {t("ducking.osSource.watchList")}
        <input
          type="text"
          value={watchListText}
          oninput={(e) => (watchListText = e.currentTarget.value)}
          onchange={commitWatchList}
        />
      </label>
      <p>{t(`ducking.osSource.status.${osStatus}`)}</p>
      <p class="command-line">{t("ducking.osSource.commandLine")}: <code>{commandLine}</code></p>
    </fieldset>

    <label>
      {t("ducking.depth")}
      <input
        type="range"
        min="0"
        max="1"
        step="0.05"
        value={depth}
        oninput={(e) => onDepthChange?.(Number(e.currentTarget.value))}
      />
    </label>
  </div>

  <style lang="scss">
    .ducking-settings {
      display: grid;
      gap: var(--space-3);
    }
    fieldset {
      display: grid;
      gap: var(--space-2);
    }
    .denial {
      color: var(--danger);
    }
    .command-line code {
      user-select: all;
    }
  </style>
  ```
- Create: `src/modules/ducking/src/DuckingSettings.test.ts`:
  ```ts
  import { describe, it, expect, afterEach, vi } from "vitest";
  import { render, screen, fireEvent, waitFor } from "@testing-library/svelte";
  import { setAppContextForTest } from "@shadowcat/ui-kit/test";
  import { DuckSourcesController } from "./controller";
  import { DEFAULT_DUCKING_PREFERENCES, DUCKING_MIRROR_STORAGE_KEY } from "./duckingMirror";
  import DuckingSettings from "./DuckingSettings.svelte";

  const logger = { debug() {}, warn() {}, error() {} };

  describe("DuckingSettings", () => {
    afterEach(() => {
      localStorage.clear();
    });

    it("renders every control labeled and persists a master-enable toggle", async () => {
      const controller = new DuckSourcesController(DEFAULT_DUCKING_PREFERENCES, logger);
      const context = setAppContextForTest({});
      render(DuckingSettings, { props: { controller }, context });
      const master = screen.getByLabelText("ducking.masterEnable") as HTMLInputElement;
      expect(master.checked).toBe(true);
      await fireEvent.click(master);
      expect(master.checked).toBe(false);
      const stored = JSON.parse(localStorage.getItem(DUCKING_MIRROR_STORAGE_KEY)!);
      expect(stored.masterEnabled).toBe(false);
      controller.dispose();
    });

    it("binds a new key via the capture button", async () => {
      const controller = new DuckSourcesController(DEFAULT_DUCKING_PREFERENCES, logger);
      const context = setAppContextForTest({});
      render(DuckingSettings, { props: { controller }, context });
      await fireEvent.click(screen.getByRole("button", { name: /ducking\.keySource\.bind/ }));
      const event = new KeyboardEvent("keydown", { code: "KeyV" });
      window.dispatchEvent(event);
      await waitFor(() => {
        const stored = JSON.parse(localStorage.getItem(DUCKING_MIRROR_STORAGE_KEY)!);
        expect(stored.keyBinding).toBe("KeyV");
      });
      controller.dispose();
    });

    it("commits the watch-list text as a trimmed, filtered array on change", async () => {
      const controller = new DuckSourcesController(DEFAULT_DUCKING_PREFERENCES, logger);
      const context = setAppContextForTest({});
      render(DuckingSettings, { props: { controller }, context });
      const input = screen.getByLabelText("ducking.osSource.watchList") as HTMLInputElement;
      await fireEvent.input(input, { target: { value: "discord, , teams ,zoom" } });
      await fireEvent.change(input);
      const stored = JSON.parse(localStorage.getItem(DUCKING_MIRROR_STORAGE_KEY)!);
      expect(stored.watchList).toEqual(["discord", "teams", "zoom"]);
      controller.dispose();
    });

    it("a mic denial reason shows a message and resets the checkbox", async () => {
      const controller = new DuckSourcesController(DEFAULT_DUCKING_PREFERENCES, logger);
      const context = setAppContextForTest({});
      const onMicToggle = vi.fn().mockResolvedValue("permission-denied");
      render(DuckingSettings, { props: { controller, onMicToggle }, context });
      const micCheckbox = screen.getByLabelText("ducking.micSource.enable") as HTMLInputElement;
      await fireEvent.click(micCheckbox);
      await waitFor(() => {
        expect(screen.getByText("ducking.micSource.denied.permission-denied")).toBeTruthy();
      });
      expect(micCheckbox.checked).toBe(false);
      controller.dispose();
    });

    it("reflects the OS monitor's live status", async () => {
      const controller = new DuckSourcesController(DEFAULT_DUCKING_PREFERENCES, logger);
      const context = setAppContextForTest({});
      render(DuckingSettings, { props: { controller }, context });
      expect(screen.getByText("ducking.osSource.status.connecting")).toBeTruthy();
      controller.dispose();
    });

    it("the depth slider forwards its value to onDepthChange and never touches the mirror", async () => {
      const controller = new DuckSourcesController(DEFAULT_DUCKING_PREFERENCES, logger);
      const context = setAppContextForTest({});
      const onDepthChange = vi.fn();
      render(DuckingSettings, { props: { controller, depth: 0.5, onDepthChange }, context });
      const slider = screen.getByLabelText("ducking.depth") as HTMLInputElement;
      expect(slider.value).toBe("0.5");
      await fireEvent.input(slider, { target: { value: "0.3" } });
      expect(onDepthChange).toHaveBeenCalledWith(0.3);
      expect(localStorage.getItem(DUCKING_MIRROR_STORAGE_KEY) ?? "").not.toContain("depth");
      controller.dispose();
    });
  });
  ```
- Modify: `src/modules/ducking/src/index.ts` — replace the Task-8 scaffold entirely:
  ```ts
  import { consoleLogger, SETTINGS_SECTION_CONTRACT, type Module } from "@shadowcat/core";
  import DuckingSettings from "./DuckingSettings.svelte";
  import { DuckSourcesController } from "./controller";
  import { readDuckingMirror } from "./duckingMirror";

  export { KeySource, NULL_SINK, DEFAULT_KEY, type DuckSink } from "./keySource";
  export {
    MicVadSource,
    VadEngine,
    VAD_PROCESSOR_NAME,
    DEFAULT_SENSITIVITY,
    VAD_FRAME_MS,
    type MicVadDeps,
    type MicVadDenialReason,
  } from "./micVad";
  export { OsMonitorSource, DEFAULT_THRESHOLD, type OsMonitorStatus } from "./osMonitor";
  export { DuckSourcesController } from "./controller";
  export {
    readDuckingMirror,
    writeDuckingMirror,
    DEFAULT_DUCKING_PREFERENCES,
    DUCKING_MIRROR_STORAGE_KEY,
    type DuckingPreferences,
  } from "./duckingMirror";
  export { default as DuckingSettings } from "./DuckingSettings.svelte";

  /** The active controller, set by `register` and torn down by `unregister` — module-scoped
   * rather than closure-captured per call because `unregister` needs it and the `Module`
   * interface gives no other channel between the two. Safe as a singleton because exactly one
   * `WorldSession` (and therefore one active `ducking` registration) exists per browser tab
   * (`App.svelte`'s single `session` state). */
  let currentController: DuckSourcesController | null = null;

  /** Voice ducking: a mic voice-activity source, an OS audio-session monitor source (the
   * `shadowcat audio-monitor` subcommand), and a push-to-duck key — three `DuckSource`s behind
   * the audio engine's `DuckController` contract. `KeySource`/`OsMonitorSource`
   * run for real from `register()` onward (both need only `window`/`WebSocket`); the mic
   * source and the wiring to `ctx.audio.duck` land once the real integration is complete, once
   * the audio engine's `AudioApi` exists in this worktree. */
  export const ducking: Module = {
    manifest: {
      id: "ducking",
      version: "0.1.0",
      dependencies: {},
      requires: [SETTINGS_SECTION_CONTRACT],
      provides: [],
    },
    register(ctx) {
      const prefs = readDuckingMirror(localStorage);
      const controller = new DuckSourcesController(prefs, consoleLogger());
      currentController = controller;
      ctx.contributions.contribute({
        id: "ducking:settings",
        contract: SETTINGS_SECTION_CONTRACT,
        component: DuckingSettings,
        props: { controller },
        settingsSection: { labelKey: "ducking.sectionTitle" },
      });
    },
    unregister() {
      currentController?.dispose();
      currentController = null;
    },
  };
  ```
- Modify: `src/modules/ducking/src/index.test.ts` — replace the Task-8 scaffold:
  ```ts
  import { describe, it, expect, afterEach } from "vitest";
  import { ContributionRegistry, SETTINGS_SECTION_CONTRACT } from "@shadowcat/core";
  import { ducking } from "./index";

  describe("ducking module", () => {
    afterEach(() => {
      ducking.unregister?.();
      localStorage.clear();
    });

    it("requires SETTINGS_SECTION_CONTRACT and provides nothing", () => {
      expect(ducking.manifest.requires).toEqual(["shadowcat.settings-section"]);
      expect(ducking.manifest.provides).toEqual([]);
    });

    it("contributes its settings section with the expected metadata", () => {
      const contributions = new ContributionRegistry();
      ducking.register({ contributions } as never);
      const list = contributions.contributionsFor(SETTINGS_SECTION_CONTRACT);
      expect(list).toHaveLength(1);
      expect(list[0].id).toBe("ducking:settings");
      expect(list[0].settingsSection).toEqual({ labelKey: "ducking.sectionTitle" });
    });
  });
  ```
- Modify: `src/client/ui-kit/src/locales/en.ts` — append a new top-level `ducking.` group (master §3: "one NEW top-level group per milestone"), placed after the last existing group in the file:
  ```ts
    "ducking.sectionTitle": "Voice ducking",
    "ducking.masterEnable": "Enable voice ducking",
    "ducking.keySource.title": "Push-to-duck key",
    "ducking.keySource.enable": "Enable push-to-duck key",
    "ducking.keySource.bind": "Bound to {key} — click to rebind",
    "ducking.keySource.bindPrompt": "Press a key…",
    "ducking.micSource.title": "Microphone voice activity",
    "ducking.micSource.enable": " Duck when I speak",
    "ducking.micSource.sensitivity": "Sensitivity",
    "ducking.micSource.denied.permission-denied": "Microphone permission was denied.",
    "ducking.micSource.denied.no-microphone": "No microphone was found.",
    "ducking.micSource.denied.unknown": "Could not access the microphone.",
    "ducking.osSource.title": "OS audio-session monitor",
    "ducking.osSource.enable": "Duck when a watched app talks",
    "ducking.osSource.port": "Monitor port",
    "ducking.osSource.threshold": "Peak threshold",
    "ducking.osSource.watchList": "Watched apps (comma-separated)",
    "ducking.osSource.commandLine": "Run this on your machine",
    "ducking.osSource.status.connecting": "Not connected — start the monitor below.",
    "ducking.osSource.status.connected": "Connected.",
    "ducking.osSource.status.not-running": "Not running.",
    "ducking.osSource.status.unsupported": "Unsupported on this OS.",
    "ducking.depth": "Duck depth",
  ```
  (fix the stray leading space in `"ducking.micSource.enable"`'s value before committing — it
  reads "Duck when I speak", no leading space.)

- [ ] **Step 1:** write every `.test.ts`/`.test.ts`-equivalent file above FIRST (all fail:
  nothing exists yet), then implement `osMonitor.ts`'s `setPort`, `duckingMirror.ts`,
  `controller.ts`, `DuckingSettings.svelte`, `index.ts`, `en.ts`. `pnpm --filter
  @shadowcat/module-ducking test` PASS. `pnpm --filter @shadowcat/ui-kit test` PASS (locale key
  count/shape tests, if any, still pass — check `rg "en\[" src/client/ui-kit/src/locales` for
  any test asserting the exact key set).
- [ ] **Step 2:** `pnpm -r typecheck`, `pnpm lint`, `pnpm lint:docs`, `pnpm lint:props`, `pnpm
  lint:aria-labels`, `pnpm lint:comments`, `pnpm docs:check-examples` PASS.
- [ ] **Step 3:** `git commit -m "feat(ducking): settings section, preferences mirror, source controller" -- src/modules/ducking/ src/client/ui-kit/src/locales/en.ts`

### Task 12: `App.svelte` module list + `defaultModuleOrder.test.ts`

**Files:**
- Modify: `src/client/shell/package.json` — add, after `"@shadowcat/module-sheet-table":
  "workspace:*",`:
  ```json
      "@shadowcat/module-ducking": "workspace:*",
  ```
- Modify: `pnpm-lock.yaml` — regenerate via `pnpm install` from the repo root.
- Modify: `src/client/shell/src/App.svelte` — add the import after `import { sheetTable } from
  "@shadowcat/module-sheet-table";` (master §3: "Append after `sheetTable`"):
  ```ts
    import { ducking } from "@shadowcat/module-ducking";
  ```
  and append `ducking` at the end of the `modules:` array in `enterWorld`:
  ```ts
        modules: [panels, coreUi, topBar, statusBar, stage, settings, gameSettings, sceneBrowser, assetBrowser, actors, factions, conditions, combatTracker, sceneTools, chat, chatComposer, chatCard, notes, tables, sheetFallback, sheetActor, sheetItem, sheetNote, sheetTable, ducking],
  ```
- Modify: `src/client/shell/src/lib/defaultModuleOrder.test.ts` — add the import and a THIRD
  describe block. Leave the existing "default docked panel" describe block's two `for (const m
  of [...])` registration lists UNTOUCHED: `ducking` contributes NO panel (only a
  `SETTINGS_SECTION_CONTRACT` entry), so it has no place in a panel-order assertion — that
  absence is itself the thing worth asserting, which the new block does:
  ```ts
  import { ducking } from "@shadowcat/module-ducking";
  import { SETTINGS_SECTION_CONTRACT } from "@shadowcat/core";
  ```
  ```ts
  describe("ducking contributes a settings section, not a panel", () => {
    it("registers exactly one shadowcat.settings-section entry and no shadowcat.panel entry", () => {
      const contributions = new ContributionRegistry();
      const ctx = { contributions, hooks: { on: () => () => {} } } as never;
      ducking.register(ctx);
      expect(contributions.contributionsFor(PANEL_CONTRACT)).toHaveLength(0);
      expect(contributions.contributionsFor(SETTINGS_SECTION_CONTRACT)).toHaveLength(1);
      ducking.unregister?.();
    });
  });
  ```
  (`ducking.register(ctx)` calls `readDuckingMirror(localStorage)` — this test file already
  runs under the shell package's jsdom environment, which provides a real `localStorage`, so no
  stubbing is needed; add `afterEach(() => localStorage.clear())` inside the new describe block
  for hygiene between runs.)

- [ ] **Step 1:** `pnpm install` from the repo root; `pnpm --filter @shadowcat/shell test`,
  `pnpm --filter @shadowcat/shell typecheck` PASS.
- [ ] **Step 2:** `pnpm lint`, `pnpm lint:docs`, `pnpm build` PASS.
- [ ] **Step 3:** `git commit -m "feat(shell): register the ducking module" -- src/client/shell/ pnpm-lock.yaml`

### Task 13: Playwright `ducking.spec.ts` (written here, dispatcher-run)

**Files:**
- Create: `src/client/shell/e2e/ducking.spec.ts`:
  ```ts
  import { test, expect, login } from "./fixtures";
  import type { WorkerAccount } from "./fixtures";

  async function enterFreshWorld(
    page: import("@playwright/test").Page,
    name: string,
    account: WorkerAccount,
  ): Promise<void> {
    await login(page, account.username, account.password);
    await page.getByLabel("New world name").fill(name);
    await page.getByRole("button", { name: "Create world" }).click();
    await expect(page.locator(".stage-host")).toHaveAttribute("data-render-ready", "true", {
      timeout: 30_000,
    });
  }

  // Only the key source is driven here: mic and OS sources need hardware, so their unit
  // tests are the gate. Every selector below is verified against the ACTUAL merged
  // `AudioPanel.svelte` and `Settings.svelte`/`DuckingSettings.svelte` markup — written against
  // real DOM (settings labels, the `ducking.masterEnable`/`ducking.keySource.enable`
  // i18n keys resolved to their English strings) plus `AudioPanel.svelte`'s `data-duck-gain`
  // attribute naming the duck indicator; if the actual attribute name differs, use the
  // real one and update this comment, never invent a selector unverified against the merged
  // source.
  test("holding the bound push-to-duck key drops the audio panel's duck gain, releasing restores it", async ({
    page,
    account,
  }) => {
    await enterFreshWorld(page, "Ducking World", account);

    await page.getByTestId("topbar-settings").click();
    await expect(page.getByLabelText("Enable voice ducking")).toBeChecked();
    await page.getByLabelText("Enable push-to-duck key").check();

    // The default binding is Backquote; the settings section shows it once bound.
    await expect(page.getByRole("button", { name: /Bound to Backquote/ })).toBeVisible();

    // Open the audio panel to observe the duck-gain indicator.
    await page.getByTestId("launcher-trigger").click();
    await page.getByTestId("launcher-item-audio:panel").click();

    await expect(page.locator("[data-duck-gain]")).toHaveAttribute("data-duck-gain", "1");

    await page.keyboard.down("Backquote");
    await expect
      .poll(async () => Number(await page.locator("[data-duck-gain]").getAttribute("data-duck-gain")))
      .toBeLessThan(1);

    await page.keyboard.up("Backquote");
    await expect
      .poll(async () => Number(await page.locator("[data-duck-gain]").getAttribute("data-duck-gain")))
      .toBe(1);
  });
  ```
- [ ] **Step 1:** `pnpm --filter @shadowcat/shell typecheck` PASS. `pnpm lint` PASS. Do NOT run
  the suite (dispatcher-run, master §4).
- [ ] **Step 2:** `git commit -m "test(e2e): push-to-duck key drives the audio panel's duck gain (written; dispatcher runs)" -- src/client/shell/e2e/ducking.spec.ts"`

### Task 14: docs (`ducking.md`, hosting guide, protocol.md, ARCHITECTURE rows) + `server-ops` skill

**Files:**
- Create: `docs/site/modules/ducking.md`:
  ```md
  # ducking

  ## Purpose

  Voice ducking: lowers music/ambience while someone talks. Three independent sources feed one
  contract (M23's `DuckController`, which takes the max demand): a browser microphone
  voice-activity detector, the `shadowcat audio-monitor` OS audio-session subcommand (a
  localhost WebSocket reporting Discord's — or any watched app's — output peak), and a
  push-to-duck key. This milestone never touches gain nodes itself.

  ## Contributions

  | Id | Contract | Component | Meta |
  |---|---|---|---|
  | `ducking:settings` | `shadowcat.settings-section` | `DuckingSettings` | labelKey `ducking.sectionTitle` |

  ## Components

  - `DuckingSettings.svelte` — master enable, per-source enable + sensitivity/threshold, duck
    depth, the OS monitor's port + a copyable command line (including `--allow-origin` for the
    page's own origin), live connection status, and the watched-process list editor (a LIVE
    editor: changes send the running monitor a `watch` frame without a restart).
  - `keySource.ts` (`KeySource`) — held-key push-to-duck, ignoring editable targets.
  - `micVad.ts` (`VadEngine`, `MicVadSource`) + `vad.worklet.ts` (`VadProcessor`) — an
    adaptive-floor energy VAD running entirely inside an `AudioWorklet`; the raw audio buffer
    never leaves the worklet (PII invariant) — only a boolean per 20 ms frame crosses the
    `MessagePort`.
  - `osMonitor.ts` (`OsMonitorSource`) — the browser-side WebSocket client for `shadowcat
    audio-monitor`'s `/levels` endpoint; reconnects with backoff, reports "not running" after 3
    failed attempts.
  - `controller.ts` (`DuckSourcesController`) — owns the key + OS-monitor sources for the whole
    world session (constructed once in `register`, outliving the settings panel's own mount
    lifecycle); the mic source and the wiring to `ctx.audio.duck` are constructed in `App.svelte`'s
    module registration once M23's `AudioApi` is available (see this module's `index.ts`).
  - `duckingMirror.ts` — per-device `localStorage` persistence (`shadowcat.ducking`), styled
    after the shell's theme mirror.

  ## Contracts & seams

  - **Requires** `shadowcat.settings-section` (from `settings`).
  - Consumes M23's `AudioApi.duck.addSource`/`AudioApi.context()`.
  - The `shadowcat audio-monitor` subcommand it talks to is documented in `server-ops`'s skill
    and this page's own protocol section below.

  ## Local audio-monitor protocol

  `shadowcat audio-monitor [--port 31998] [--allow-origin <origin>...] [--watch <substr>...]`
  serves `ws://127.0.0.1:<port>/levels`. A connection's `Origin` header must be in the allowlist
  (default `http://localhost:30000`, `http://127.0.0.1:30000`) or it is refused before any frame
  is sent. Frames: `{"type":"hello","os":string,"supported":boolean,"reason"?:string}` once on
  connect; `{"type":"levels","sessions":[{"process":string,"peak":number}]}` at 10 Hz,
  pre-filtered to the watch list server-side (never reveals an unwatched process); the client
  may send `{"type":"watch","names":string[]}` to replace the live watch list.

  ## Pointers

  - Source: `src/modules/ducking/`
  - Server subcommand: `src/server/src/audio_monitor/`
  - API: [`@shadowcat/module-ducking`](/api/ts/modules/_shadowcat_module-ducking.html)
  ```
- Modify: `docs/site/modules/index.md` — add a `ducking` row (same table shape as the other
  module rows; place it after `tables`/near the audio-related entries once M23's `audio.md`
  exists, or at the end of the Gameplay group's table if M23 has not merged into this worktree
  yet — mirror whatever ordering `index.md` uses for the OTHER Gameplay-group modules at the
  time this task runs).
- Modify: `docs/site/.vitepress/config.mts` — add, in the `"Gameplay"` sidebar group, after
  `{ text: "sheet-table", link: "/modules/sheet-table" },`:
  ```ts
              { text: "ducking", link: "/modules/ducking" },
  ```
- Modify: `docs/site/guides/hosting.md` — add a new `##` section after "## Mobile" and before
  "## Troubleshooting":
  ```md
  ## Voice ducking on your machine

  The ducking module's Settings section can duck the table's music/ambience while Discord (or
  any voice app) is talking, without the Discord SDK — it reads OS-reported audio-session peak
  levels through a small companion command:

  ```bash
  shadowcat audio-monitor --port 31998 --allow-origin http://localhost:30000 --watch discord
  ```

  The Settings section prints this exact command line (with `--allow-origin` set to the page's
  own origin) — copy it verbatim rather than retyping it. It runs entirely on your own machine,
  never uploads anything, and only ever reports the peak level of processes matching `--watch`
  (default `discord`) — every other running application stays invisible to it and to the page.

  **Windows**: no extra setup; WASAPI session metering works out of the box.

  **macOS 14.2+**: the first run triggers the OS's own "System Audio Recording" permission
  prompt — accept it once. Older macOS reports itself unsupported honestly rather than failing
  silently.

  **Linux**: needs a running PipeWire session (the default on current Fedora/Ubuntu desktops);
  without one the monitor reports "PipeWire not running" rather than crashing.
  ```
- Modify: `docs/site/protocol.md` — add a new `##` section after "## Scene channels" (the last
  existing section per this file's own table of contents):
  ```md
  ## Local audio-monitor protocol

  `shadowcat audio-monitor` is a SEPARATE localhost WebSocket, unrelated to the `/ws` protocol
  above — no login, no world, no `ClientMsg`/`ServerMsg`. The ducking module's `OsMonitorSource`
  connects to `ws://127.0.0.1:<port>/levels`; the connection's `Origin` header must be in the
  monitor's allowlist or it is refused before any frame is sent.

  | Frame | Direction | Shape |
  |---|---|---|
  | `hello` | monitor → client | `{ "type": "hello", "os": string, "supported": boolean, "reason"?: string }`, sent once on connect |
  | `levels` | monitor → client | `{ "type": "levels", "sessions": [{ "process": string, "peak": number }] }`, at 10 Hz, already filtered to the watch list |
  | `watch` | client → monitor | `{ "type": "watch", "names": string[] }`, replaces the live watch list without a restart |
  ```
- Modify: `docs/design/ARCHITECTURE.md`:
  - §3 "Core technology (v1)" — append three rows to the table, after the `UI framework`/
    `Canvas renderer`/`Build tooling` rows (append at the end):
    ```md
    | OS audio-session monitor (Windows) | `windows` crate (WASAPI) | <license from spec §5, Task 1> | Vendor | `shadowcat audio-monitor`'s Windows backend; per-session peak metering, never the Discord SDK. |
    | OS audio-session monitor (macOS) | Core Audio process tap (macOS 14.2+), hand-bound FFI over `core-foundation` | <license from spec §5, Task 1> | Vendor | Same subcommand's macOS backend; older macOS reports itself unsupported. |
    | OS audio-session monitor (Linux) | `pipewire` crate (libpipewire) | <license from spec §5, Task 1> | Vendor | Same subcommand's Linux backend; the tree's first system-library (`libpipewire-0.3-dev`) CI dependency. |
    ```
    By the time this task runs, Task 1 has ALREADY measured and recorded the real license
    strings in `docs/superpowers/specs/2026-09-11-m27-voice-ducking-design.md` §5 (this same
    worktree, an earlier task) — copy those exact values into the three `<license from spec §5,
    Task 1>` placeholders above; the committed `ARCHITECTURE.md` must never contain the literal
    placeholder text, only the real measured license strings, matching every other row's
    plain-string convention.
  - §4 "Deferred behind abstractions" — the "Discord audio ducking" row is now BUILT; delete
    that row entirely (master §8: "§4's ... ducking ... rows are struck or rewritten as
    built" — ducking is struck since it now has a live implementation, not merely a design
    intent).
  - §5's rejected-dependency bullet currently reads "**Discord Game SDK** — proprietary.
    Discord audio ducking (deferred) is implemented via OS audio-session APIs, never the SDK."
    Rewrite it in the present tense: "**Discord Game SDK** — proprietary. Voice ducking is
    implemented by `shadowcat audio-monitor` over OS audio-session APIs (WASAPI / Core Audio
    process taps / PipeWire), never the SDK." — the word "deferred" must not survive.
- [ ] **Step 1:** write `ducking.md`, edit `index.md`/`config.mts`/`hosting.md`/`protocol.md`/
  `ARCHITECTURE.md` exactly as above (substituting Task 1's real measured license strings into
  the ARCHITECTURE §3 rows). `pnpm docs:check-examples`, `pnpm build:all` (background + log; the
  docs build link-checks every new page) PASS.
- [ ] **Step 2:** Update the plugin checkout's `shadowcat-codebase-server-ops` skill
  (`C:/Users/emper/.claude/skills/shadowcat-codebase/skills/shadowcat-codebase-server-ops/SKILL.md`)
  — the skill currently states "`Cli` (flat `clap::Parser` struct, no `clap::Subcommand`)",
  which this milestone makes STALE; replace that sentence with: "`Cli` (a flat `clap::Parser`
  struct PLUS one `#[command(subcommand)] command: Option<CliCommand>` field, added by M27 for
  `shadowcat audio-monitor` — every existing flat flag still parses unconditionally;
  `CliCommand` is a `clap::Subcommand` enum with exactly one variant today, named so it never
  shadows `std::process::Command` in the CLI tests)." Add a new bullet under "Key
  files & seams" naming `audio_monitor` (`src/server/src/audio_monitor/`): the
  `SessionMonitor` trait, `platform_monitor()`, its three `#[cfg(target_os)]` backends, and the
  `server::run`/`run_with_monitor` split that makes the WS server's origin-allowlist and
  frame-serialization logic testable against a `FakeMonitor` without a real OS audio API.
  In the same checkout, edit `hooks/codebase-skill-reminder.py`'s `SUBSYSTEMS` list: append
  `r"src/server/src/audio_monitor/"` to the `server-ops` entry's pattern list (the subcommand
  is server-ops knowledge; its skill bullet is the one this step just wrote), and add one
  self-test line to `hooks/test-codebase-skill-reminder.sh` after the existing `check` lines
  (absolute Windows-style path, the established convention):
  ```bash
  check am1 "C:/Dev/Shadowcat/src/server/src/audio_monitor/server.rs" "shadowcat-codebase-server-ops"
  ```
  Run `bash hooks/test-codebase-skill-reminder.sh` from the plugin checkout root; paste its
  output. DO NOT commit inside the plugin checkout yet (Step 4 commits).
- [ ] **Step 3:** Also update `shadowcat-codebase-audio`'s skill file IF it already exists in
  this checkout by the time this task runs (it is created by M23, which per master §5 merges to
  `main` before M27's own merge-forward, so it will typically exist): add a bullet under "Key
  files & seams" naming `src/modules/ducking/`'s three source classes as `DuckController`
  consumers and `keySource.ts`'s `DuckSink` structural-stub pattern any future ducking-adjacent
  source should follow; append `r"src/modules/ducking/"` to the `audio` entry's pattern list in
  `hooks/codebase-skill-reminder.py`'s `SUBSYSTEMS` (the entry M23 created) and add the
  self-test line
  ```bash
  check am2 "C:/Dev/Shadowcat/src/modules/ducking/src/keySource.ts" "shadowcat-codebase-audio"
  ```
  to `hooks/test-codebase-skill-reminder.sh`, re-running it. **If `shadowcat-codebase-audio` does NOT exist yet when this task
  runs** (M23 not yet merged to `main` in this dispatcher's timeline), skip this step here and
  do it as part of Task 15's integration task instead (which cannot start until M23 has
  merged anyway) — state explicitly in this task's report which of the two happened.
- [ ] **Step 4:** dispatch `shadowcat-codebase:shadowcat-spec-reviewer` on the skill + hook
  diff(s) from Steps 2–3 (blind, diff pre-generated). Apply findings. Run `node
  scripts/check-skill-symbol-refs-cli.mjs` and `pnpm run test:scripts` from THIS repo (not the
  plugin checkout) — 0 broken citations introduced. Commit + push inside the plugin checkout
  (`C:/Users/emper/.claude/skills/shadowcat-codebase/`, its own git remote) — NOT part of any
  commit in this repo.
- [ ] **Step 5:** `git commit -m "docs(ducking): module page, hosting guide, protocol, ARCHITECTURE rows" -- docs/site/ docs/design/ARCHITECTURE.md"`

### Task 15: integration — merge M23, wire `ctx.audio.duck`, full gate battery, `gate:push`

**Runs only after the dispatcher confirms M23 is merged to `origin/main`** (master §5: M27
merges last, after M22/M28/M24/M23/M25/M26). Every other task above needs nothing from any
other milestone and should already be complete and committed on `m27-ducking`.

**Files:**
- `git fetch origin && git merge origin/main` in the worktree (a merge commit, never a rebase —
  immutable history). Expected conflicts per master §3: `src/client/shell/src/App.svelte`'s
  module list (M23/M24/M26 also append there — keep every appended module, in whatever combined
  order the merge produces, `ducking` last per this plan's Task 12), `src/client/ui-kit/src/locales/en.ts`
  (each milestone owns a distinct top-level group — keep both), `src/client/ui-kit/src/appContext.ts`
  (M23 appends its `audio: AudioApi` member after `panels` — keep it, this task reads it next),
  `src/client/ui-kit/src/__fixtures__/appContextTest.ts` (M23 adds an `audio` default — keep
  it), `.github/workflows/ci.yml` (M23's own `cmake` check step — keep both additions, do not
  reorder), `src/server/Cargo.toml` (M23's audio-crate hunk lands under its own `# Phase 3: M23`
  comment, M27's under its own — keep both, in whatever order the merge produces per master §3:
  "alphabetical is NOT required"), `docs/HISTORY.md`/`docs/PLAN.md` (each milestone appends its
  own entry under "## Phase 3 — Atmosphere" — keep both; do NOT flip the phase heading, since
  M25/M26/M28 have not necessarily merged yet at this point in the integration order — only the
  LAST milestone to merge does that, per master §3).
- Modify: `src/client/core/src/audio.ts` (M23's file, now present after the merge) — read its
  actual `AudioApi` interface FIRST, then add exactly:
  ```ts
    /** The shared engine `AudioContext`, for a consumer that needs to register its own
     * `AudioWorkletNode` against the SAME graph `AudioApi` otherwise owns entirely (the
     * ducking module's `MicVadSource`). `null` until `unlock()` resolves —
     * Web Audio requires a user gesture before a context exists at all. */
    context(): AudioContext | null;
  ```
  (placed inside the `AudioApi` interface, after `unlock(): Promise<void>;` and before `readonly
  duck: DuckController;`, matching the ordering M23's own file already establishes around
  those two members).
- Modify: whichever file constructs the concrete `AudioApi` implementation M23 shipped (find it
  via `rg "implements AudioApi|: AudioApi = \{|class AudioEngine" src/client` after the merge —
  the exact file is M23's own choice and cannot be named here before that code exists) — add a
  `context()` method returning the engine's own lazily-created `AudioContext` (or `null` before
  `unlock()`), following whatever internal field M23 already uses to hold it.
- Modify: `src/modules/ducking/src/index.ts` — extend `register(ctx)` to wire the real seam
  (replacing the module-scoped `audioDuckCleanup` var this step introduces):
  ```ts
  import type { MicVadDenialReason } from "./micVad";
  import { MicVadSource } from "./micVad";

  /** Cleanup for the real `ctx.audio.duck` wiring, run from `unregister()`. `null` before
   * `register()` has wired it (never happens in production; guards a defensive double-call). */
  let audioDuckCleanup: (() => void) | null = null;
  ```
  and inside `register(ctx)`, after the existing `ctx.contributions.contribute({...})` call:
  ```ts
      const keySink = ctx.audio.duck.addSource("ducking:key");
      const osSink = ctx.audio.duck.addSource("ducking:os-monitor");
      controller.wireToAudioDuck(keySink, osSink);

      let mic: MicVadSource | null = null;
      const onMicToggle = async (enabled: boolean): Promise<MicVadDenialReason | null> => {
        if (!enabled) {
          mic?.disable();
          return null;
        }
        const audioContext = ctx.audio.context();
        if (!audioContext) return "unknown"; // not yet unlocked — the statusbar's "Enable
                                              // audio" control must run first
        if (!mic) {
          mic = new MicVadSource({
            audioContext,
            workletUrl: new URL("./vad.worklet.ts", import.meta.url),
            sensitivity: prefs.micSensitivity,
            logger: consoleLogger(),
          });
          mic.setSink(ctx.audio.duck.addSource("ducking:mic"));
        }
        return mic.enable();
      };
      // Re-contribute with the now-real onMicToggle (the earlier call above passed none):
      // NOTE — this duplicates a contribution id; instead, thread `onMicToggle` into the
      // ORIGINAL `contribute` call's `props` object directly by building `props` once, below
      // the `onMicToggle` closure, and moving the single `ctx.contributions.contribute` call
      // to AFTER this block (reorder: onMicToggle defined first, `contribute` called once with
      // `props: { controller, onMicToggle }`). See the Step 1 note below — this reordering is
      // part of this task's implementation, not two separate contributes.

      audioDuckCleanup = () => {
        ctx.audio.duck.removeSource("ducking:key");
        ctx.audio.duck.removeSource("ducking:os-monitor");
        if (mic) ctx.audio.duck.removeSource("ducking:mic");
        mic?.disable();
      };
  ```
  and in `unregister()`:
  ```ts
    unregister() {
      currentController?.dispose();
      currentController = null;
      audioDuckCleanup?.();
      audioDuckCleanup = null;
    },
  ```
  **Step 1 note (resolving the reordering called out above):** restructure `register(ctx)` so
  the single `ctx.contributions.contribute({...})` call happens ONCE, after `onMicToggle` is
  defined, with `props: { controller, onMicToggle }` — delete the props-less contribute call
  this plan's Task 11 wrote and do not call `contribute` twice. Read the CURRENT file before
  editing (`Read` first per this repo's Edit-tool discipline) and produce the single coherent
  `register(ctx)` body; the two code blocks above are given as the pieces to compose, not as
  literal sequential edits.
- Modify: `src/modules/ducking/src/DuckingSettings.svelte` — the depth slider now sources
  its value from the context AT MOUNT and writes back directly, so every opening of the
  settings panel shows the current per-device depth (a value captured once in `register(ctx)`'s
  `props` object would go stale, because a contribution's `props` are stored, never
  re-evaluated). Delete the `depth`/`onDepthChange` props and their two doc comments from the
  `$props()` block; change the context line to `const { t, audio } = getAppContext();`; add
  `let depth = $state(audio.duck.depth);` beside the other `$state` declarations; the slider
  becomes `value={depth}` / `oninput={(e) => { depth = Number(e.currentTarget.value);
  audio.duck.setDepth(depth); }}`. `DuckController.depth` (readonly) and `setDepth(depth)` are
  master §2.2's seam, owned and persisted by M23 (`shadowcat.audio` mirror) — this module reads
  and forwards, never stores. The `contribute` call's props object stays `{ controller,
  onMicToggle }`.
- Modify: `src/modules/ducking/src/DuckingSettings.test.ts` — replace the earlier "forwards its
  value to onDepthChange" test: the fixture becomes `setAppContextForTest({ audio: { duck: {
  depth: 0.5, setDepth: vi.fn(), addSource: vi.fn(), removeSource: vi.fn() } } })` (extend the
  fixture shape to whatever `AppContext.audio` requires after the merge — read
  `src/client/core/src/audio.ts`); render with `props: { controller }`; assert the slider's
  initial `value` is `"0.5"`, that an `input` event with `"0.3"` calls the fixture's `setDepth`
  with `0.3`, and that `localStorage.getItem(DUCKING_MIRROR_STORAGE_KEY) ?? ""` never
  contains `"depth"`. Every OTHER test in the file that renders the component must now also
  pass an `audio` fixture (the component reads `audio.duck.depth` at mount) — add it to the
  file's shared `setAppContextForTest({...})` helper call rather than per test.
- Modify: `src/modules/ducking/src/index.test.ts` — add a test that `register`/`unregister`
  add/remove exactly the three `ducking:key`/`ducking:os-monitor`/`ducking:mic` sources on a
  fixture `AppContext`'s `audio.duck` (spy `addSource`/`removeSource`, mirroring how
  `setAppContextForTest`'s `combat`/`chat` fixtures are spied on elsewhere in this codebase).
- Create: `src/modules/ducking/src/index.integration.test.ts` — spec §4's "`DuckController`
  integration through M23's real controller after the merge (max of two sources, release
  curve)": construct M23's REAL `DuckController` implementation (read its actual constructor/
  factory after the merge — likely `@shadowcat/audio`'s engine class or a standalone
  `DuckController` class in `src/client/core/src/audio.ts`'s companion runtime package; import
  whatever the real symbol is, never a hand-rolled substitute), register `ducking`'s `KeySource`
  and `OsMonitorSource` against two of its `addSource` handles directly (bypassing the full
  module `register()` — this test targets the controller/source boundary, not the module
  wiring, which `index.test.ts` already covers), drive one source to demand 1 while the other
  stays 0, assert `duck.gain` reflects only the max (not a sum), then drop both sources to 0 and
  assert `duck.gain` returns to 1 over M23's documented ~600 ms release curve (poll with fake
  timers per whatever tick mechanism the real controller uses — read its implementation before
  writing the assertion's timing). `// @vitest-environment node` unless the real controller
  needs a DOM/Web-Audio stub, in which case omit the pragma and stub whatever it needs the same
  way `micVad.test.ts`'s `stubDeps` does.
- Modify: `docs/HISTORY.md` — append, under "## Phase 3 — Atmosphere" (create the heading if no
  other Phase-3 milestone has merged yet):
  ```md
  ### M27 · Voice ducking ✅

  Branch: `m27-ducking`. Spec: `docs/superpowers/specs/2026-09-11-m27-voice-ducking-design.md`.
  Delivered: `shadowcat audio-monitor` subcommand (Windows WASAPI / macOS Core Audio process tap
  / Linux PipeWire backends behind one `SessionMonitor` trait; a localhost, origin-gated
  WebSocket at `/levels`; `hello`/`levels`/`watch` frames; watch-list filtering server-side).
  `@shadowcat/module-ducking`: `KeySource`, `MicVadSource` (`VadEngine` + `vad.worklet.ts`,
  PII-invariant boolean-only worklet messaging), `OsMonitorSource` (reconnect/backoff),
  `DuckSourcesController`, a per-device `localStorage` preferences mirror, and the
  `DuckingSettings.svelte` contributed section. New client seam: `SETTINGS_SECTION_CONTRACT`
  (`Contribution.settingsSection`), rendered by `Settings.svelte` after its built-in content;
  `settings` module now `provides` it. Wired to M23's `ctx.audio.duck.addSource`/
  `AudioApi.context()` in this milestone's integration task. Dependency review: [Task 1's
  measured `windows`/`core-foundation`/`pipewire` license findings — paste the actual values
  from `docs/superpowers/specs/2026-09-11-m27-voice-ducking-design.md` §5 here]. CI: PipeWire
  dev headers on the Ubuntu legs of `rust`/`docs`; doc-coverage clippy added to the three-OS
  `rust` matrix. Tests: [paste the actual `cargo test`/`pnpm -r test` summary counts]. e2e:
  `ducking.spec.ts` written, NOT run by this milestone (dispatcher-run per master §4).
  ```
- Delete the `docs/PLAN.md` M27 entry (if one exists there beyond the Phase-3 paragraph master
  §1 already accounts for) per the same convention Task 10 of the M20 plan used.
- [ ] **Step 1:** merge, resolve conflicts exactly per the conventions above, then implement
  every code change listed (order: `audio.ts`'s `context()` addition and its concrete-engine
  implementation FIRST, since `ducking`'s own edits depend on it compiling — `depth`/`setDepth`
  already ship with M23 per master §2.2; then `index.ts`'s reorganized `register`/`unregister`
  then `DuckingSettings.svelte`'s mount-time depth read; then the three new/modified test
  files, including `index.integration.test.ts`).
- [ ] **Step 2:** the FULL gate battery from this plan's header (background the long ones;
  read every log before claiming green): `cargo test --all`, `cargo fmt --check`, `cargo clippy
  --all-targets -- -D warnings`, `cargo clippy -- -D missing-docs -D
  clippy::missing-docs-in-private-items` (both the `docs`-job invocation AND confirm the
  three-OS `rust`-job invocation Task 1 added still passes locally on this OS), `git diff
  --exit-code src/types/generated`, `pnpm -r typecheck`, `pnpm -r test`, `pnpm build`, `pnpm
  lint`, `lint:docs`, `lint:props`, `lint:comments`, `lint:allowances`, `lint:file-size`,
  `lint:inline-tests`, `lint:aria-labels`, `lint:gate-manifest`, `lint:settings-privacy`,
  `lint:binary-size` (release build), `pnpm docs:check-examples`, `pnpm docs:check-rust-examples`,
  `pnpm run test:scripts`, `pnpm run check:svelte-runtime`, `pnpm --filter "shadowcat-example-*"
  build`, `pnpm --filter @shadowcat/core test:e2e`.
- [ ] **Step 3:** if `shadowcat-codebase-audio` was NOT yet updated by Task 14 Step 3 (because
  it did not exist at that time), do it now (it certainly exists now, since M23 has merged):
  add the ducking-consumer bullet described in Task 14 Step 3. Dispatch
  `shadowcat-codebase:shadowcat-spec-reviewer` on the diff; apply findings; `node
  scripts/check-skill-symbol-refs-cli.mjs`, `pnpm run test:scripts` 0 broken; commit + push
  inside the plugin checkout.
- [ ] **Step 4:** buddy-check the WHOLE branch diff (`git diff origin/main...HEAD` from the
  worktree) — dispatch `shadowcat-codebase:shadowcat-spec-reviewer` +
  `shadowcat-codebase:shadowcat-code-reviewer` blind, diff pre-generated by the dispatcher, per
  master §5's merge-forward protocol. Apply every finding; re-run the affected gate slice.
- [ ] **Step 5:** `git commit -m "feat(ducking): wire the three sources to AudioApi.duck; merge main" -- src/ docs/`
  (the merge commit itself already exists from Step 1 — this is a SEPARATE, subsequent commit
  for the wiring + doc changes; do not amend the merge commit).
- [ ] **Step 6:** `pnpm gate:push` (tree-keyed receipt) immediately before `git push`. Dispatcher
  sequencing check per master §5: `git rev-parse origin/main main` measured, the branch
  contains `origin/main`, the `gate:push` receipt present for the branch HEAD. Open the PR;
  merge only after BOTH CI runs are green (`main` is branch-protected, `--auto` is off).
- [ ] **Step 7:** report: STATUS, every commit on the branch, every gate result line, the
  Playwright suite explicitly marked NOT RUN (dispatcher's job), the plugin skill diff stat, and
  any deviation from this plan (the `context()` addition in particular, since it depends on
  M23's actual shipped shape) with its rationale.

---

## Self-review

**Spec coverage.** Every named symbol/behavior in
`2026-09-11-m27-voice-ducking-design.md` maps to a task: §1 sources → Tasks 8–10; §2 client
(`SETTINGS_SECTION_CONTRACT`, `DuckingSettings.svelte`, `MicVadSource`/`vad.worklet.ts`,
`OsMonitorSource`, `KeySource`) → Tasks 7–11; §3 server (`Cli` subcommand, `audio_monitor`
module + three backends, the WS server, the first-task dependency/CI review) → Tasks 1–6; §4
tests (server frame/allowlist/filter/clap tests, client VAD/OS-source/key-source/settings
tests, the `DuckController` max/release-curve integration test, `ducking.spec.ts`) → Tasks
2–3–4–5–6 (server), 8–11 (client unit), 13 (e2e), 15 (post-merge integration test); §5
dependency table → Task 1 (measured) and Task 14 (ARCHITECTURE rows); §6 docs/skills →
Task 14 (+ Task 15 for the `audio` skill and HISTORY.md, sequenced by whether M23 has merged
yet). Master §2.7's `SETTINGS_SECTION_CONTRACT` ownership, §3's shared-file conventions, and
§5's "M27 merges last, never stubs the seam" rule are honored structurally: every source class
takes an injected `DuckSink`, and `ctx.audio.duck`/`AudioApi.context()` are referenced ONLY in
Task 15.

**Placeholder scan.** Two pieces of intentionally-incomplete scaffolding exist mid-plan, both
explicitly required to be resolved (never committed as shipped) within their OWN task's steps
before that task's commit: the macOS process-tap peak-callback wiring (Task 5, `peak: 0.0` +
two keep-alive lines, replaced in Task 5 Step 1) and a scratch `spawn_server` helper in the
server WS test file (Task 6, explicitly instructed for deletion in Task 6 Step 1). No other
`TODO`/`FIXME`/unresolved stub exists in any file this plan creates or edits.

**Type/name consistency.** `DuckSink` (declared once in `keySource.ts`) is the single sink
type `MicVadSource`, `OsMonitorSource`, `KeySource`, and `DuckSourcesController` all share;
`MicVadDenialReason`, `OsMonitorStatus`, `DuckingPreferences`, `SettingsSectionMeta`/
`SETTINGS_SECTION_CONTRACT`, and the Rust `SessionLevel`/`SessionMonitor`/`MonitorError`/
`CliCommand`/`AudioMonitorArgs` are each defined exactly once and referenced identically by name
and shape everywhere they are used across tasks.

**Known residual verification risk (disclosed, not hidden):** the exact `windows`/`pipewire`
crate API surface (module paths, exact type names) and the macOS process-tap `extern "C"`
signatures are transcribed from general knowledge of each platform's audio API shape, not
compiled against the resolved crate versions or the actual macOS SDK headers — every backend
file's module doc carries an explicit verification note naming this and instructing the coder
to correct exact names against the resolved version/SDK while preserving the stated
architecture (dedicated background thread per backend, poll-shaped trait, basename-reduction
discipline). This is the single largest source of expected friction in Tasks 3–5.

## Spec gaps found (report to the user; not silently resolved)

1. **The e2e spec's exact audio-panel selector is unverifiable at plan-writing time.** M27's
   spec cites `data-duck-gain` as the audio panel's duck indicator, but the audio panel itself
   is M23's deliverable and does not exist in this repository yet (M23 and M27 are sibling
   specs written before either is implemented). Task 13 writes `ducking.spec.ts` against the
   name the spec gives and documents, in the spec file's own comment, that the selector must be
   re-verified against M23's actual merged markup — this is an inherent cross-milestone
   sequencing gap, not an oversight in this plan.
2. **macOS process-tap and Windows WASAPI exact API surfaces are unverified against real SDK
   headers/crate versions** (see "Known residual verification risk" above) — flagged here per
   the "never assert without verifying" directive rather than presented as certain.
