# M23 — Audio — Design Spec

> Master: `2026-09-11-phase3-master-integration-design.md` (§2.2 the seam this milestone
> owns; §7 the dependency rulings; §9 D2, D3, D7, D9). Two sub-projects in ONE worktree,
> sequential: **M23a** mixer + channels + playlists + world-clock sync + transcode + duck
> bus; **M23b** spatial emitters + wall occlusion. Sound/VFX PLAYBACK was scoped to Phase 3 by
> M18 (which shipped `SoundEmission`); this is that playback.

## 1. Shape of the system

```
GM transport intent ──► server `audio::transport` ──► `audio-state` doc (server-stamped started_at)
                                                        │ broadcast (per-recipient, standard)
scene/token/wall edits ──► `"audibility"` derived channel ─┐
                                                        ▼
client `@shadowcat/audio` AudioEngine: TrackPlayers (sync to server clock) + EmitterPlayers
   (gains from audibility) ──► channel GainNodes ──► duck GainNode ──► master ──► destination
```

The client never decides WHAT plays (a document does) or HOW LOUD an emitter is heard (the
server does); it decides only its own device's channel gains and mute state.

## 2. Server — M23a

### 2.1 Engine doc types (`data::engine::audio` — new module `src/server/src/data/engine/audio.rs`)

```rust
/// doc_type "playlist" — standard write rules (owner + WRITE_FIELDS), not GM-only.
PlaylistEngine { tracks: Vec<PlaylistTrack>, mode: PlaylistMode, channel: AudioChannel,
                 fade_ms: u32 /* crossfade, 0..=10_000 */ }
PlaylistTrack  { asset: String, name: Option<String>, gain: f64 /* 0..=1 */, loop_: bool /* serde "loop" */ }
PlaylistMode   = enum { Sequential, Shuffle, LoopAll, Single }      // serde snake_case
AudioChannel   = enum { Music, Ambience, Sfx }                      // "master"/"ui" are client-only buses

/// doc_type "audio-state" — one per world (seeded by `world_seed` beside the registries), the
/// server's own transport writes only (`WriteOrigin::AudioTransport`, a new variant beside
/// `CombatTransition`). The origin restriction is enforced where the ONE existing precedent
/// is: `apply_intent`'s Create/Update/Delete dispatch checks the STORED doc_type against the
/// origin (the `WriteOrigin::ConfigSeed` guard on `system-defaults`); `audio-state` gets the
/// symmetric guard there (only `AudioTransport` may write it). `validate_engine`/`normalize_
/// engine` never see the origin, so this is NOT a validate-arm rule. Every recipient's copy
/// is therefore authoritative and joiners hear the table.
AudioStateEngine { playing: Vec<PlayingTrack>, shuffle_seed: u64 }
PlayingTrack { id: Uuid, playlist: Option<Uuid>, track_index: u32, asset: String,
               channel: AudioChannel, gain: f64, loop_: bool,
               started_at: i64 /* server ms when position 0 was at */,
               paused_at: Option<i64> /* server ms; Some ⇒ paused */ }
```

- `ENGINE_DOC_TYPES` gains `"playlist"`, `"audio-state"` (append, master §3); `validate_engine`/
  `normalize_engine` arms; ts-rs regen. `PlaylistEngine::validate`: track count ≤ 512,
  non-empty asset ids, finite gains, `fade_ms` cap. `AudioStateEngine::validate`: ≤ 16 playing.
- `SceneEngine.ambience: Option<SceneAmbience { playlist: Uuid, gain: f64 }>` (appended field,
  `#[serde(default)]`): when the world's active scene changes (`world-settings.activeScene`
  write commits), `audio::transport::on_active_scene` stops the previous scene's ambience
  entries (`playlist == old.ambience.playlist`) and plays the new one — one server rule, no
  client involvement. GM local roam (`viewedSceneId` ≠ active) does not change audio.
- `WorldSettingsEngine.audio: Option<AudioOverlay>` (appended): `spatial: Option<bool>`
  (default true), `occlusion: Option<Occlusion>` (`Walls` default | `None`),
  `through_wall_gain: Option<f64>` (default 0.25 — an occluded emitter is muffled, not
  silent, so a player still learns something is behind the door, which is the UX invariant
  11 asks for). Edited in `game-settings`' world-defaults editor (a new "Audio" fieldset).

### 2.2 Transport intent (`ws::protocol`, appended)

```rust
ClientMsg::AudioTransport { op: AudioOp }
AudioOp = enum (tag "type", snake_case) {
  Play   { playlist: Option<Uuid>, asset: Option<String>, track_index: Option<u32>, channel: Option<AudioChannel>, gain: Option<f64>, loop_: Option<bool> },
  Pause  { id: Uuid }, Resume { id: Uuid }, Stop { id: Uuid }, StopAll,
  Seek   { id: Uuid, position_ms: u64 }, Next { id: Uuid }, Prev { id: Uuid },
  SetGain{ id: Uuid, gain: f64 },
}
ServerMsg::AudioError { reason: String }          // refusal (authz, cap, unknown id)
```

- Handler `audio::transport::handle_transport` (new module `src/server/src/audio/`): GM-only
  (`WorldRole::Gm`, fail closed), per-user rate bucket (ScenePing pattern), loads the
  `audio-state` doc, applies the op purely (`audio::state::apply(&AudioStateEngine, op, now,
  &playlist_lookup) -> Result<AudioStateEngine, AudioError>` — a pure function with its own
  tests), commits through `Room::commit_ops_locked` under `WriteOrigin::AudioTransport`.
  `Play` from a playlist resolves the track by `mode` (`Shuffle` uses `shuffle_seed` — a
  deterministic order every client can reproduce for the "up next" display).
- Track END is client-observed but server-decided: when a non-looping track's duration elapses
  the client sends `AudioOp::Next { id }`; the server checks `now ≥ started_at + duration_ms`
  (from the asset row, §2.3) before advancing — the first client's report wins, later ones are
  no-ops because the entry `id` already changed. No client can skip a track early.

### 2.3 Transcode pipeline (`data::asset::process` audio arm; master §7, D9)

- `AssetKind::Audio` (`content_type` starts with `audio/`); the `kind` filter and the browser's
  kind chips learn it. Assets table gains `duration_ms INTEGER NULL` and `sample_rate INTEGER
  NULL` (edit `migrations/0001_init.sql` in place — no migration files pre-customers).
- `process_staged`'s new arm for `audio/*` (sniffed by `symphonia`'s probe, never trusted from
  the client's `Content-Type` alone): decode → mono/stereo f32 at source rate → `rubato`
  (MIT) resample to 48 kHz → `opus` encode (VBR, 96 kbps stereo / 64 kbps mono, 20 ms frames)
  → `ogg` pages → `<uuid>.opus.ogg` (`Variant::Opus`, appended to `Variant` and
  `SIBLING_SUFFIXES`). **Audio deliberately does NOT mirror the image pipeline's canonical
  SWAP** (where the converted bytes become the canonical file and the original moves to the
  GM-only `.orig` sibling served by `/original`): for audio the CANONICAL file stays the
  uploaded original (member-readable through the normal serve route, because it is every
  non-GM player's playback fallback) and Opus is a derivative SIBLING like `.thumb.webp`,
  served through `?variant=opus`. `retain_originals` is not consulted for audio. The
  derivative is produced at commit time or not at all: over-cap input (duration > 30 min or
  decoded frames > 2^28 samples) or a decode failure stores the original with the explicit
  tag `audio:untranscoded` and NO derivative, and `?variant=opus` on an asset without the
  sibling answers 404 — there is NO lazy on-demand regeneration path for audio (the
  thumb/preview `write_derivatives` regenerate-on-serve convention is cheap for an image
  resize and unacceptable for a minutes-long transcode); `reconvert` is the only way to retry.
- Build toolchain: `opus` binds libopus through `audiopus_sys`, whose build script drives
  `cmake` (unlike `libwebp-sys`, which uses the `cc` crate — so the webp precedent proves
  only that a C compiler is present, not cmake). GitHub-hosted runner images for all three
  OSes ship `cmake`; **the plan's first task runs `cmake --version` on each matrix leg (a
  throwaway CI step, or a check inside the existing rust job) and records the result here
  before the dependency lands.** If any leg lacks it, add the `lukka/get-cmake` step to that
  leg — never a hand-installed toolchain.
- `GET /api/worlds/{world}/assets/{id}?variant=opus` serves the derivative (the `serve`
  route's `ServeQuery` variant match gains an `"opus"` arm beside `"thumb"`/`"preview"`);
  `reconvert` accepts audio assets (its `original_retained` precondition holds by
  construction for audio).
- Client `AssetResolver.audioUrl(id): { primary: string; fallback: string; primaryType:
  "audio/ogg; codecs=opus" }` and the engine picks `primary` iff
  `HTMLMediaElement.canPlayType(primaryType) !== ""`. **The plan's first task is a probe that
  records `canPlayType` for `audio/ogg; codecs=opus`, `audio/webm; codecs=opus` on Chromium,
  Firefox and WebKit (Playwright's three engines) into the spec's §2.3 as measured text** — the
  container choice (Ogg vs WebM) is decided by that measurement, not assumed.

### 2.4 World seed / bundle

`world_seed` creates the `audio-state` singleton; `world_bundle` export/import carries
`playlist` docs and audio assets with their derivatives (the pipeline's existing per-asset
sibling handling covers `SIBLING_SUFFIXES`).

## 3. Client — M23a

### 3.1 `@shadowcat/audio` — new package `src/client/audio/` (framework-neutral, no Svelte)

- `AudioEngine(opts: { resolver: AssetResolver; serverNow: () => number; channels?: … })`:
  one `AudioContext` created lazily on `unlock()`; graph `master ← duck ← {music, ambience,
  sfx, ui}` GainNodes; `setChannel`, `mute`, `unlock` (resumes the context inside the calling
  gesture), `dispose`.
- `TrackPlayer`: `HTMLAudioElement` + `MediaElementAudioSourceNode` (streams long music, no
  decode-to-RAM); `sync(entry: PlayingTrack, serverNow)` computes `pos = (serverNow −
  started_at) / 1000` (paused ⇒ `(paused_at − started_at)`), seeks when `|currentTime − pos|
  > 0.25 s`, rate-nudges ±2 % inside that window (no audible seek for small drift),
  crossfades on entry replacement over the playlist's `fade_ms`.
- `OneShotPlayer`: decoded-buffer LRU (≤ 32 MiB) + `AudioBufferSourceNode`; `play(asset,
  {channel, gain})` for `AudioApi.playOneShot`.
- `DuckController`: max over sources' demands, smoothed with attack 50 ms / release 600 ms
  through the duck GainNode: `gain = 1 − demand × depth` where `depth` is per-device (default
  0.7, exposed as `readonly depth` + `setDepth(depth)` per master §2.2, persisted in the
  `shadowcat.audio` mirror beside the channel gains — M27's settings section drives it) and
  `duckable` channels default to music + ambience.
- `applyState(state: AudioStateEngine)`: diff `playing` by `id`; create/sync/stop players.
  Called from the store subscription on every `audio-state` change and on a 1 Hz tick for
  drift.
- Every node creation is guarded for a `null` context (audio not unlocked ⇒ state is tracked,
  nothing plays, and unlock catches up by calling `applyState` again).

### 3.2 Shell + AppContext

`AppContext.audio: AudioApi` (master §2.2) backed by one `AudioEngine` per world session,
`serverNow` = the EXISTING public `WsClient.serverNow()` (the `TimePong`-calibrated clock;
the private `serverOffsetMs` field stays private — nothing new is exposed). Per-device channel gains/mutes and duck depth persist in
`localStorage` under `shadowcat.audio` (D1's reasoning: per device), via a
`readAudioMirror`/`writeAudioMirror` pair beside the theme mirror. The statusbar gains a
speaker control (`src/modules/statusbar/src/AudioUnlock.svelte`): shows "Enable audio" until
`unlock()` resolves, then a mute toggle.

### 3.3 Modules

- `src/modules/audio/` (`@shadowcat/module-audio`): the **Audio panel** — channel sliders +
  mutes (device), "Now playing" list with transport buttons (GM: play/pause/stop/next/prev/
  seek slider; players: read-only list), playlists list with live search (`docTypes:
  ["playlist"]`, the `TablesPanel` pattern), per-row Play, Create (`canCreate("playlist")`),
  Delete (`canDelete`), open sheet. Panel `order` after tables.
- `src/modules/sheet-playlist/` (`@shadowcat/module-sheet-playlist`): the playlist sheet —
  name, mode, channel, fade, a whole-array `tracks` editor (the `TableSheet` rows pattern:
  every mutation replaces the array on `change`), track pick through `ctx.pickAsset({ kind:
  "audio" })` (the pick overlay learns the `audio` kind chip + an inline preview button).
- `game-settings`: the Audio fieldset (§2.1's overlay); scene overrides editor gains the
  `ambience` picker (playlist search + gain).

## 4. M23b — Spatial emitters + wall occlusion

### 4.1 Server — the `"audibility"` derived channel (`scene::audibility`, new module)

Per recipient, per subscribed scene:

```rust
AudibilityFrame { scene: Uuid, listener: Option<Listener { x, y, elevation }>,
                  emitters: Vec<AudibleEmitter { token: Uuid, asset: String, gain: f64, pan: f64,
                                                 loop_: bool, x: f64, y: f64 }> }
```

- Listener = the recipient's primary listener token: the first token (by id) in the scene the
  recipient effectively owns that has a `sight_sources` entry, else their first owned token,
  else `None`. A GM without a token gets `listener: None` and EVERY emitter at `gain = 1.0,
  pan = 0` (the GM hears the whole table by default — a per-device "hear as token" switch in
  the panel makes the GM's listener a chosen token: `AudioApi.listenAs(token | null)` sends
  `ClientMsg::AudioListenAs { token }`, stored per-connection like `SceneSubscribe` state).
- `gain = falloff(d / radius_world) × occlusion`, `falloff(t) = clamp(1 − t², 0, 1)` (inverse
  square shaped, 0 at the radius edge), `radius_world = SoundEmission.radius × cell size` from
  `scene_grid_sizes` (the ONE grid-size source, never a default), `× SoundEmission.volume`;
  `occlusion` = 1 when no `los` wall whose elevation band contains BOTH the listener's and the
  emitter's elevation intersects the listener→emitter segment. ONE shared function — a new
  `scene::audibility::segment_occluded(walls, from, to, (e_listener, e_emitter))` composed
  from the two primitives that already exist: `scene::segments_cross` (the segment
  intersection the raycaster uses) and `scene::elevation::wall_occludes` (the elevation-band
  test); an anti-drift test pins that the raycaster and `segment_occluded` agree on the same
  wall set (mutating either side's band test fails it). Else `through_wall_gain`.
  `Occlusion::None` ⇒ always 1.
- `pan = clamp((emitter.x − listener.x) / radius_world, −1, 1)`.
- Emitters below `gain < 0.005` are still SENT with their gain (invariant 11: sent-then-quiet;
  the client also learns a sound exists to pre-buffer it) — but an emitter on a token the
  recipient may not READ is omitted (permissions, not secrecy-by-audio).
- Recomputed with `"vision"`'s triggers (token move commit, wall/light/scene edits, emission
  edits) — hooks into the same invalidation `compute_derived` subscribers use. Move-stream
  frames do not update audibility mid-walk; the client interpolates gains over 300 ms.
- `world-settings.audio.spatial == false` ⇒ every emitter at `gain = volume, pan = 0`.

### 4.2 Client

- `EmitterPlayer` (in `@shadowcat/audio`): per `token` a looping `AudioBufferSourceNode` (or
  one-shot when `loop_` is false — fires once per rising edge of "present in the frame",
  tracked by token id) → `StereoPannerNode` → `GainNode` (ramped to the frame's gain over
  300 ms) → the `sfx` channel. `PerformanceSettings.spatialAudio == false` ⇒ pan 0, gain =
  `volume` (the frame's gain ignored — a device choice, not a secrecy leak: the frame still
  arrives).
- `WorldSession` subscribes `"audibility"` beside `"footprints"` for the viewed scene;
  `AudioEngine.applyAudibility(frame)`.
- Audio panel gains the GM's "Listen as" token picker.

## 5. Tests

- Server: `audio::state::apply` truth table (every op, caps, unknown id, paused/resume timing,
  shuffle determinism from seed); transport handler authz (player refused, GM accepted, rate
  bucket); `on_active_scene` swaps ambience; transcode: a WAV synthesized in the test (440 Hz,
  2 s) → derivative exists, `duration_ms` ≈ 2000, Ogg pages parse, over-cap input → tagged
  pass-through; `AssetKind::Audio` sniff on a WAV/MP3 header vs a mislabelled PNG; audibility:
  falloff at 0/half/edge/beyond, occluded by an in-band wall, not by an out-of-band wall,
  `through_wall_gain`, `Occlusion::None`, listener selection order, GM null listener, unreadable
  token omitted, `spatial=false`; `segment_occluded` parity with the raycaster (mutate one and
  the test fails); world seed creates `audio-state`; import/export round-trips a playlist.
- Client `@shadowcat/audio` (node env with a stub `AudioContext`): sync math (pos, pause,
  drift threshold, rate nudge), `applyState` diff (start/stop/replace/crossfade), duck max +
  attack/release, LRU eviction, emitter rising-edge one-shot, gain ramp, `canPlayType`
  selection (stubbed both ways), `listenAs` frame.
- Modules: panel (GM vs player controls, live search, create/delete gates), sheet (tracks
  editor whole-array writes, OCC `old`), settings fieldsets.
- e2e `audio.spec.ts` (written here; dispatcher-run; Playwright's Chromium launched with
  `--autoplay-policy=no-user-gesture-required` in `playwright.config.ts`): GM uploads the
  test WAV, creates a playlist, plays → the player's audio panel shows the track and the
  stage's `data-audio-playing` count is `1`; GM pauses → `0`; the player cannot see transport
  buttons.

## 6. Docs + skills

- `docs/site/modules/audio.md`, `sheet-playlist.md`; `docs/site/protocol.md` rows for
  `audio_transport`/`audio_error`/`audio_listen_as` and the `audibility` channel; the hosting
  guide's asset section mentions Opus derivatives; ARCHITECTURE §3 rows (symphonia, opus,
  rubato, ogg) and §4's audio/asset-conversion rows rewritten as built.
- New skill `shadowcat-codebase-audio` (master §6); updates to `assets`, `scene-rendering`,
  `documents-permissions`, `realtime-sync`; hook map globs `src/client/audio/`,
  `src/modules/audio/`, `src/modules/sheet-playlist/`, `src/server/src/audio/`,
  `src/server/src/data/engine/audio`, `src/server/src/scene/audibility`.
- `docs/HISTORY.md` M23 entry.
