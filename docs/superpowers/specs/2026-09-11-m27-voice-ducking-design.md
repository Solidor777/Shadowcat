# M27 — Voice ducking — Design Spec

> Master: `2026-09-11-phase3-master-integration-design.md` (§2.2 the `DuckController`
> contract M23 owns and this milestone drives; §7 the OS-monitor dependency ruling; §9 D7).
> Goal: when people talk, the music gets out of the way — on every platform, without the
> Discord SDK. Three sources, one contract.

## 1. Sources

| Source | Where it runs | Signal | Platforms |
|---|---|---|---|
| **Mic voice activity** (`MicVadSource`) | the browser | energy-based VAD on the user's own microphone | every platform including Android/iOS |
| **OS audio-session monitor** (`OsMonitorSource`) | `shadowcat audio-monitor` subcommand on the user's machine, localhost WebSocket | peak level of watched processes (Discord, any voice app) | Windows (WASAPI), macOS 14.2+ (Core Audio process tap), Linux (PipeWire) |
| **Push-to-duck key** (`KeySource`) | the browser | a held key | everywhere |

Each is a `DuckSource` registered on `ctx.audio.duck` (`addSource(id).set(level)`); the
controller takes the max; this milestone never touches gain nodes itself.

## 2. Client — `src/modules/ducking/` (`@shadowcat/module-ducking`)

- **Settings section.** A new `SETTINGS_SECTION_CONTRACT` (`shadowcat.settings-section`,
  multi). Following the `PanelMeta`/`SheetMeta` precedent, the per-family metadata is a new
  optional field on `Contribution` — `settingsSection?: { labelKey: string }` — beside the
  existing top-level `component`/`order`; `src/modules/settings/src/Settings.svelte` renders
  every contribution to it after its built-in content (the extension seam for any module's
  settings, invariant 7). The ducking module contributes `DuckingSettings.svelte`: a master
  enable, per-source enable + sensitivity, duck depth (forwards to M23's per-device `depth`),
  the OS monitor's port + the exact command line to run (copyable, including the
  `--allow-origin` for the page's own origin), status ("connected", "not running",
  "unsupported on this OS" as the monitor reports), and the watched-process list editor — a
  LIVE editor: on change the client sends the monitor a `{ "type": "watch", "names":
  [...] }` frame (§2's `OsMonitorSource` → §3's protocol), so the running monitor filters on
  the new list without a restart; the `--watch` flag only seeds the initial list. Per-device
  persistence in `localStorage` `shadowcat.ducking`.
- **`MicVadSource`** (`src/modules/ducking/src/micVad.ts` + `vad.worklet.ts`): on enable,
  `getUserMedia({ audio: { echoCancellation: true, noiseSuppression: true } })` → an
  `AudioWorkletNode` computing RMS per 20 ms frame with an adaptive noise floor (EMA of the
  quietest 10 % of the last 2 s); speech = RMS > floor × sensitivity for 3 consecutive frames;
  hangover 400 ms; demand `1` while speaking else `0`. **Privacy invariant (ironclad — PII):
  the audio never leaves the worklet; no buffer is stored, transmitted or exposed; the worklet
  posts only a boolean per frame.** The permission prompt fires only on enable; a denial
  shows the reason and leaves the source off. Runs on its own `AudioContext`? No — it uses the
  engine's context (`AudioApi` gains `context()` for worklet registration in the integration
  task; before M23 merges the source is tested against a stub).
- **`OsMonitorSource`** (`osMonitor.ts`): `WebSocket` to `ws://127.0.0.1:<port>/levels`;
  frames `{ "type": "levels", "sessions": [{ "process": string, "peak": number }] }` at 10 Hz
  and `{ "type": "hello", "os": string, "supported": boolean, "reason"?: string }` on connect;
  demand = 1 when any watched process's `peak > threshold` (default 0.02) with the same 400 ms
  hangover; reconnects with backoff; "not running" shown after 3 failed attempts.
- **`KeySource`**: `keydown`/`keyup` on a configurable key (default `Backquote`), ignoring
  events whose target is an editable element.

## 3. Server — `shadowcat audio-monitor` subcommand

- `Cli` gains a clap `#[command(subcommand)] command: Option<Command>` with
  `Command::AudioMonitor(AudioMonitorArgs { #[arg(long, default_value_t = 31998)] port: u16,
  #[arg(long)] allow_origin: Vec<String>, #[arg(long)] watch: Vec<String> })`; the root flags keep working unchanged (no subcommand ⇒ serve). The
  existing one-shot branches (`backup_to`, `restore_from`) are untouched.
- `src/server/src/audio_monitor/` (`mod.rs` + `windows.rs` + `macos.rs` + `linux.rs`, each
  `#[cfg(target_os = …)]`, plus `fake.rs` under `#[cfg(test)]`):

  ```rust
  pub struct SessionLevel { pub process: String, pub peak: f32 }
  pub trait SessionMonitor: Send { fn poll(&mut self) -> Result<Vec<SessionLevel>, MonitorError>; }
  pub fn platform_monitor() -> Result<Box<dyn SessionMonitor>, MonitorError>  // Unsupported on old macOS
  ```

  - Windows: `windows` crate — `IMMDeviceEnumerator` → default render endpoint →
    `IAudioSessionManager2::GetSessionEnumerator` → per session `IAudioSessionControl2::
    GetProcessId` + `IAudioMeterInformation::GetPeakValue`; process name via
    `QueryFullProcessImageNameW` reduced to its BASENAME (`Path::file_name`) before it is
    stored in `SessionLevel.process` — the full path never leaves the backend. Runs on a
    dedicated thread with COM initialized (MTA).
  - macOS: Core Audio process tap (`AudioHardwareCreateProcessTap` + an aggregate device with
    the tap, `AudioObjectGetPropertyData(kAudioHardwarePropertyProcessObjectList)` for pids →
    names via `proc_pidpath`, likewise reduced to the basename); on macOS < 14.2
    `platform_monitor` returns
    `MonitorError::Unsupported("macOS 14.2 or newer")` and the hello frame says so. Requires
    the user to grant the "System Audio Recording" permission on first run (the prompt is
    macOS's; the docs page explains it).
  - Linux: `pipewire` crate — connect to the session's PipeWire core, enumerate nodes with
    `media.class == "Stream/Output/Audio"`, and read `application.process.binary`; the peak is
    measured by linking a passive capture stream to each node's monitor port. PipeWire's API
    is event-loop driven, so the backend runs the loop on its own thread and publishes the
    latest per-node peak into shared state (`Arc<Mutex<…>>`) that `SessionMonitor::poll`
    reads at 10 Hz — the trait stays poll-shaped for all three backends. When no PipeWire
    socket exists the hello frame reports `supported: false, reason: "PipeWire not running"`.
    **CI: this is NEW infrastructure** — `.github/workflows/ci.yml` has no OS package step
    today; the `ubuntu-latest` legs of the `rust` and `docs` jobs gain
    `sudo apt-get install -y libpipewire-0.3-dev` (pkg-config discovery), the first
    system-library dependency in the tree (`webp`/`opus` build their own vendored C).
  - The WS server: axum on `127.0.0.1` only; a connection is accepted only when its `Origin`
    header is in `allow_origin` (default: `http://localhost:30000` and `http://127.0.0.1:30000`
    — `Config::default()`'s bind is `127.0.0.1:30000`; the settings UI prints the exact
    `--allow-origin` for the origin the page runs on). An unlisted origin is closed before the hello frame — a local
    web page must not be able to fingerprint the user's running apps.
  - Frames: `hello` (once), `levels` (10 Hz), and the client→monitor `watch` frame that
    replaces the live watch list. Peak values are clamped to `[0, 1]`; process names are
    basenames truncated to 128 chars; the frame never carries pids, paths or window titles.
    **Filtering happens in the monitor process, before anything is sent:** only sessions
    whose basename matches the watch list (case-insensitive substring; initial list from
    `--watch`, default `discord`) are ever serialized — nothing about the user's other
    applications leaves the process.
- **First task — capability + license measurement.** Before any dependency is added: run
  `cargo tree`-equivalent license inspection for `windows`, `coreaudio-rs`/`coreaudio-sys`,
  `core-foundation`, `pipewire`/`libspa` (crates + the C libraries they bind), record the
  results in this spec's §5 table as measured, and confirm each is on the invariant-9 list.
  Then a smoke run of each backend's enumeration on the matching CI runner asserts `Ok(_)` or
  `Unsupported` — never a panic on a device-less host. **Doc coverage across all three
  backends:** the `-D missing-docs -D clippy::missing-docs-in-private-items` clippy step lives
  in the Ubuntu-only `docs` job today, so `windows.rs`/`macos.rs` would never be doc-checked;
  this milestone adds the same invocation to the three-OS `rust` matrix job (the `docs` job
  keeps its copy), so every `cfg`-gated module is held to the ratchet on the leg that
  compiles it.

## 4. Tests

- Server: hello/levels/watch frame serialization; origin allowlist (allowed / refused before
  hello); watch-list filtering (substring, case-insensitive, default `discord`, replaced live
  by a `watch` frame); basename reduction; clamping/truncation;
  `FakeMonitor` drives the 10 Hz loop; per-platform enumeration smoke test compiled and run on
  its own `cfg` (asserts no panic and a sane result on a headless runner); clap parsing (root
  flags still work; `audio-monitor --port 0` binds an ephemeral port and prints it).
- Client: VAD energy/floor/hangover state machine (node env, synthetic frames); OS source
  demand/threshold/hangover/reconnect (stub WebSocket); key source ignores editable targets;
  settings persistence; `DuckController` integration through M23's real controller after the
  merge (max of two sources, release curve).
- e2e `ducking.spec.ts` (written here; dispatcher-run): open Settings → Voice ducking → enable
  the key source → hold the key while a playlist plays → the audio panel's duck indicator
  (`data-duck-gain`) drops below `1`; release → returns to `1`. (Mic and OS sources need
  hardware; their unit tests are the gate.)

## 5. Dependency review (filled in by the first task; the master §7 row is the ruling)

Measured against the resolved `Cargo.lock` versions via `cargo metadata` (crates) and each
project's own licensing (the C libraries they bind).

| Crate | Version | License | Binds | Result |
|---|---|---|---|---|
| `windows` | 0.58.0 | MIT OR Apache-2.0 | Win32 (system) | PASS — on the invariant-9 list |
| `coreaudio-rs` / `coreaudio-sys` | 0.2.18 (sys, latest) | MIT | CoreAudio (system) | NOT USED — `coreaudio-sys` 0.2.18 does not wrap `AudioHardwareCreateProcessTap` (grep of the crate source), so `macos.rs` hand-binds the process-tap surface via `extern "C"` over `core-foundation` per Task 5 |
| `core-foundation` | 0.10.1 | MIT OR Apache-2.0 | CoreFoundation (system) | PASS — on the invariant-9 list |
| `pipewire` / `libspa` | 0.8.0 | MIT | libpipewire-0.3 (MIT) | PASS — on the invariant-9 list |

## 6. Docs + skills

- `docs/site/modules/ducking.md`; a hosting-guide section "Voice ducking on your machine" with
  the per-OS command line and the macOS permission prompt; `protocol.md` gains a
  "local audio-monitor protocol" section; ARCHITECTURE §3 rows for the three backends and §4/§5
  ducking rows rewritten as built.
- Skills: `audio` (duck sources, the settings-section contract), `server-ops` (the
  subcommand); hook globs `src/modules/ducking/`, `src/server/src/audio_monitor/`. No new
  skill (master §6).
- `docs/HISTORY.md` M27 entry.
