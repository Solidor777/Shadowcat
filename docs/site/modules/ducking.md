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
