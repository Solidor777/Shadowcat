# Phase 3 — Atmosphere — Master Integration Spec

> The one document every Phase-3 milestone spec, plan, coder and reviewer reads first. It fixes
> the milestone set, the seams milestones share, who owns each shared file, the merge order,
> the gate battery, and the campaign directives. Per-milestone shapes live in the seven
> milestone specs listed in §1; this document never restates them — it says which milestone
> owns what and how the pieces meet.

## 0. Campaign directives (verbatim — copy into every dispatched agent's first prompt)

> The iron rule is no deferrals of existing work, or new work as it comes up - we fix this now
> unless I give my EXPRESS authorization. The only exception is if a bug or to-do has a genuine
> blocker that is already logged in a milestone in PLAN.md that has not been started yet. Another
> iron clad is rule is that when faced with a design fork, determine the best long term shape in
> keeping with our plans and goals, and implement accordingly. You only need to ask me if the
> question "what is the best long term shape in keeping with our plans and goals?" is not able to
> answer the question. Churn is not a concern. This paragraph must be copied verbatim to any
> agents dispatched in this campaign.

Plus, in the same first prompt:

- **Reporting rule:** a subagent delivers its report as the Agent tool result, via `SendMessage`
  to the dispatcher, or by writing a named file; the prompt states which. An agent given a
  `name` never returns a result — omit `name` for every dispatch whose report you need.
- **Opus is banned** for every dispatch in this campaign. Coders are
  `shadowcat-codebase:shadowcat-coder` (sonnet, effort medium); reviewers are
  `shadowcat-codebase:shadowcat-spec-reviewer` + `shadowcat-codebase:shadowcat-code-reviewer`
  (sonnet, effort high). Escalation goes to the `-fable` twins, never the `-opus` twins.
- **Never end a turn to ask whether to continue.** The dispatcher runs until the campaign's
  deliverable (§8) is measured true.

## 1. Milestones

Numbering continues from Phase 2 (M21 was the last). Seven milestones, one worktree each, built
simultaneously; the integration order in §5 is the only sequencing.

| Id | Name | Spec | Plan | Branch / worktree |
|---|---|---|---|---|
| M22 | Performance settings + render budget | `2026-09-11-m22-performance-settings-design.md` | `2026-09-11-m22-performance-settings.md` | `m22-performance` / `C:/Dev/Shadowcat-m22` |
| M23 | Audio: mixer, channels, playlists, world-clock sync, transcode; spatial + occlusion; duck bus | `2026-09-11-m23-audio-design.md` | `2026-09-11-m23-audio.md` | `m23-audio` / `C:/Dev/Shadowcat-m23` |
| M24 | VFX: sprite effects, concurrent one-shots, emitter playback | `2026-09-11-m24-vfx-design.md` | `2026-09-11-m24-vfx.md` | `m24-vfx` / `C:/Dev/Shadowcat-m24` |
| M25 | Multi-level maps + portals | `2026-09-11-m25-levels-portals-design.md` | `2026-09-11-m25-levels-portals.md` | `m25-levels` / `C:/Dev/Shadowcat-m25` |
| M26 | 3D dice | `2026-09-11-m26-dice-3d-design.md` | `2026-09-11-m26-dice-3d.md` | `m26-dice-3d` / `C:/Dev/Shadowcat-m26` |
| M27 | Voice ducking: mic voice-activity source + OS audio-session monitor | `2026-09-11-m27-voice-ducking-design.md` | `2026-09-11-m27-voice-ducking.md` | `m27-ducking` / `C:/Dev/Shadowcat-m27` |
| M28 | Sandboxed third-party validators (WASM) | `2026-09-11-m28-sandboxed-validators-design.md` | `2026-09-11-m28-sandboxed-validators.md` | `m28-sandbox` / `C:/Dev/Shadowcat-m28` |

All specs under `docs/superpowers/specs/`, all plans under `docs/superpowers/plans/`.

Everything `docs/PLAN.md`'s "Phase 3 — Atmosphere" paragraph lists is covered: audio (M23),
VFX (M24), multi-level maps + portals (M25), 3D dice (M26), the ducking module (M27, split from
audio so both build at once), the parked capability Phase 3 sandbox (M28). The user-added
performance settings are M22. Nothing from that paragraph is deferred.

## 2. Shared seams — defined ONCE, owned by ONE milestone

Every seam below is introduced by exactly one milestone (the owner) and consumed by others.
A consumer never defines a private copy: it develops against the owner's declared type and
wires up in its integration task (§5) after the owner has merged. This is the never-fork rule
applied to the campaign itself.

### 2.1 `PerformanceSettings` — owner M22

```ts
// @shadowcat/core  src/client/core/src/performance.ts
export interface PerformanceSettings {
  /** Frame-rate cap for the stage ticker; "auto" resolves via `resolvePreset`. */
  fpsCap: 30 | 60 | 120 | "uncapped";
  /** Renderer resolution as a fraction of devicePixelRatio, 0.5..=1. */
  renderScale: number;
  antialias: boolean;
  /** Per-token ColorMatrix fx (tint/desaturate/highlight) — off ⇒ filters never built. */
  tokenFx: boolean;
  /** Lighting overlay quality: "full" (per-frame photometric), "static" (no sweeps), "off". */
  lighting: "full" | "static" | "off";
  /** VFX layer (M24) — off ⇒ no VfxView reconcile, no one-shot playback. */
  vfx: boolean;
  /** 3D dice overlay (M26) — off ⇒ chat card only. */
  dice3d: boolean;
  /** Spatial audio (M23b) — off ⇒ every emitter mixes flat at channel gain. */
  spatialAudio: boolean;
  /** Redraw only when something changed (dirty-flag rendering). */
  idleSkip: boolean;
  /** Honour `prefers-reduced-motion`: token tweens snap, sweeps cross-fade in one step. */
  reducedMotion: boolean;
}
export type PerformancePreset = "auto" | "mobile" | "balanced" | "quality" | "custom";
```

Exposed as `AppContext.performance: { readonly current: PerformanceSettings; preset:
PerformancePreset; set(patch: Partial<PerformanceSettings>): void; setPreset(p:
PerformancePreset): void }`. Per-DEVICE, persisted in `localStorage` (decision D1). Consumers:
M23b reads `spatialAudio`; M24 reads `vfx` + `reducedMotion`; M26 reads `dice3d` +
`reducedMotion`; M25 reads nothing. Every consumer reads it through the reactive
`ctx.performance.current` — never a cached copy.

### 2.2 `AudioApi` + `DuckController` — owner M23

```ts
// @shadowcat/core  src/client/core/src/audio.ts (types only; the engine is @shadowcat/audio)
export type AudioChannelId = "master" | "music" | "ambience" | "sfx" | "ui";
export interface DuckController {
  /** Register a ducking source; returns its handle. Any number may be active. */
  addSource(id: string): DuckSource;
  removeSource(id: string): void;
  /** The current effective duck gain 0..=1 (1 = no ducking), reactive. */
  readonly gain: number;
  /** Per-device ducking depth 0..=1 (how far a full demand pulls the gain down; default 0.7),
   *  persisted with the device's audio mirror. M27's settings section drives it. */
  readonly depth: number;
  setDepth(depth: number): void;
}
export interface DuckSource {
  /** Set this source's demand 0..=1 (1 = fully ducked). The controller takes the max. */
  set(level: number): void;
}
export interface AudioApi {
  readonly channels: Record<AudioChannelId, { gain: number; muted: boolean }>;
  setChannel(id: AudioChannelId, patch: { gain?: number; muted?: boolean }): void;
  /** Local (this device) unlock — Web Audio needs a user gesture; the shell calls it once. */
  unlock(): Promise<void>;
  readonly duck: DuckController;
  /** Play a one-shot sfx by asset id at channel gain (M24 VFX with sound, UI cues). */
  playOneShot(asset: string, opts?: { channel?: AudioChannelId; gain?: number }): void;
  /** The server clock (`WsClient.serverNow()`), for live playlist-position readouts. */
  serverNow(): number;
  /** Send a GM transport op (`AudioOp`) — the Audio panel's play/pause/seek/next controls. */
  transport(op: AudioOp): void;
}
```

Exposed as `AppContext.audio` — M23's ONE `AppContext` member; every audio operation the
panel or another module needs (clock, transport, ducking, one-shots) hangs off it, so no
second audio-flavoured `AppContext` member ever appears. Both `serverNow` and `transport`
are thin forwarders to `WsClient`. Server-side ownership: the `playlist` and `audio-state` engine
doc types, the `audio-settings` world doc, the `"audibility"` derived channel, the audio
transcode pipeline. Consumers: M24 (`playOneShot` for a VFX's paired sound), M27 (`duck`).

### 2.3 VFX playback seam — owner M24

```ts
// @shadowcat/core  src/client/core/src/vfx.ts
export interface VfxPlayRequest {
  scene: string;
  asset: string;             // spritesheet or animated-webp asset id
  x: number; y: number;      // scene units
  scale?: number; rotation?: number;
  durationMs?: number;       // omitted ⇒ one loop of the asset
  sound?: string;            // asset id played through AudioApi.playOneShot (M23) when present
}
```

`AppContext.vfx.play(req)` sends the `ClientMsg::PlayVfx` aux frame; the server rebroadcasts
`ServerMsg::Vfx` (ScenePing template, per-user rate bucket). The `VfxView` render layer also
plays `VfxEmission`s from `EffectiveActor.vfx` (M18's component model). Consumers: M25 portals
(a portal's optional `vfx` asset plays at both ends on teleport), M26 (none).

### 2.4 Scene levels — owner M25

`SceneEngine.levels: Vec<SceneLevel>`; `SceneLevel { id, name, bottom: f64, top: f64,
background: Option<String> }`; a token's level is derived from `TokenEngine::elevation` through
ONE function `scene::elevation::level_of(levels: &[SceneLevel], elevation: f64) ->
Option<&SceneLevel>` (server) mirrored by `levelOf(levels: SceneLevel[], elevation)` in
`@shadowcat/core` — the never-fork pin is a shared conformance corpus at
`src/client/core/src/__fixtures__/levels-conformance.json`, the formula corpus's shape. `AppContext.viewedLevel: string | null` +
`setViewedLevel(id)`. Consumers: M23b (audibility already uses the elevation-banded raycaster,
so it inherits level separation for free — the spec says so explicitly), M24 (a VfxPlayRequest
gains `elevation?: number` so the layer filters by level — M24 adds the field; M25 makes the
`VfxView` honour it in its integration task).

### 2.5 Roll → 3D dice seam — owner M26

`DieRecord.kind: Option<DieKind>` (a public, every-recipient field on the outcome's records —
the natural face already is; mirrored BY HAND in `chat-docs.ts`, the `dice` crate has no
ts-rs derives). The dice-3d module subscribes to the document store itself and plays every
`roll_embed`/`table_draw` outcome it has not seen (seen-set seeded from the store at mount, so
history never replays); the chat-card module has no dependency on it.
`AppContext.dice3d.roll(outcome, rollId)` exists for a system module that rolls outside chat.
No other milestone consumes it.

### 2.6 Sandbox seam — owner M28

`module.json` gains `validators?: [{ docType: string; wasm: string }]`; `InstalledModule` gains
`validators`; the server runs each enabled world's opted-in validators inside
`validate_engine_tree`'s post-image chokepoint over the `system` band ONLY. No other milestone
consumes it.

### 2.7 UI extension contracts introduced this phase — one owner each

| Contract | Owner | Rendered by | Consumers |
|---|---|---|---|
| `SCENE_TOOL_CONTRACT` (`shadowcat.scene-tool`, multi: `{ id, icon, labelKey, onSceneClick(x, y) }`) | M24 | the `scene-tools` rail | M24's FX tool; any external module |
| `STAGE_OVERLAY_CONTRACT` (`shadowcat.stage-overlay`, multi: absolutely-positioned children over the stage canvas, `pointer-events: none` by default) | M26 | `Stage.svelte` | M26's dice overlay; M25's `LevelSwitcher` may use it in its integration task or the rail — M25 decides at plan time and states which |
| `SETTINGS_SECTION_CONTRACT` (`shadowcat.settings-section`, multi; metadata as `Contribution.settingsSection?: { labelKey }` beside the existing top-level `component`/`order`, the `PanelMeta`/`SheetMeta` precedent) | M27 | `Settings.svelte` after its built-in sections | M27's ducking section. M22's `PerformanceEditor` stays a BUILT-IN section (the theme precedent), so M22 does not wait on M27 |

`WriteOrigin` gains two variants: `AudioTransport` (M23) and `Trigger` (M25); both append,
and M25's variant replaces the `CombatTransition` origin the region-trigger effects borrow.

## 3. Shared-file ownership and edit conventions

These files are touched by more than one milestone. Conflicts here are textual and expected;
the conventions make them mechanical to resolve.

| File | Milestones | Convention |
|---|---|---|
| `src/server/src/ws/protocol.rs` (`ClientMsg`/`ServerMsg`) | M23, M24, M25 | Append new variants at the END of each enum, in milestone order; one doc comment per variant. |
| `src/client/core/src/wire.ts` (Zod mirrors) | M23, M24, M25 | Append after the last existing member of the union; run `wire.test.ts`'s parity suite. |
| `src/client/core/src/ws-client.ts`, `src/client/shell/src/lib/worldSession.svelte.ts` | M23, M24, M25, M26 | New handlers as NEW methods appended after the emote handlers; never edit an existing handler's body. |
| `src/client/ui-kit/src/appContext.ts` | all | Append the new member after `panels`; each milestone owns exactly the member §2 names. |
| `src/client/ui-kit/src/__fixtures__/appContextTest.ts` + every `setAppContext(` literal | all | Add a default for your member (`rg "setAppContext\(|: AppContext =" src --type ts --type svelte -l`). |
| `src/client/shell/src/App.svelte` module list, `defaultModuleOrder.test.ts` | M23, M24, M26, M27 | Append after `sheetTable`. (M22 and M28 add no module package: M22 lives inside `settings`/`statusbar`, M28 inside `ModuleManager`.) |
| `src/client/shell/src/main.ts` | M22 (performance mirror load beside the theme mirror) | Append after the theme line. |
| `src/server/src/ws/protocol.rs` `ServerMsg::Reject`, `wire.ts` `RejectSchema`, `worldSession.svelte.ts` `onReject`, `App.svelte` toast | M28 (`detail` field) | Additive optional field; M28 alone. |
| `.github/workflows/ci.yml` | M23 (cmake check), M27 (`apt-get libpipewire-0.3-dev`, doc-coverage clippy on the matrix), M28 (`wasm32-unknown-unknown` target) | Each adds its own step; none reorders existing steps. |
| `src/client/ui-kit/src/locales/en.ts` | all | One NEW top-level group per milestone (`performance.`, `audio.`, `vfx.`, `levels.`, `dice3d.`, `ducking.`, `sandbox.`); never edit another group's keys. |
| `src/server/src/data/engine/mod.rs` (`ENGINE_DOC_TYPES`, `normalize_engine`, `validate_engine`) | M23, M25 | New doc types appended at the END of the slice; new match arms after the `"note"` arm. |
| `src/server/src/data/engine/scene.rs` (`SceneEngine`, `WorldSettingsEngine`) | M23 (`ambience`, `audio` overlay), M25 (`levels`) | Append fields at the END of the struct with `#[serde(default)]`; extend `validate` at its end. |
| `WriteOrigin` (`data::command` or wherever it is declared) and every exhaustive `match` on it | M23, M25 | Append variants; each milestone updates every match arm in its own commit. |
| `src/modules/settings/src/Settings.svelte` | M22 (built-in Performance section), M27 (contract render loop) | M22 inserts its section between Theme and Modules; M27 appends the contract loop after Modules. |
| `src/modules/stage/src/Stage.svelte` | M22 (`antialias` tracked through a `$derived` boolean so the mount `$effect` re-creates the backend only on an actual flip, the canvas element itself re-created via `{#key}`, `data-*` attrs as markup-owned reactive attributes, `performance`/`onStats` opts), M24 (`vfxAssets`, `playVfx`), M25 (level scoping, `data-level`), M26 (`STAGE_OVERLAY_CONTRACT` surface) | Each milestone adds its own block; no milestone reorders existing markup. |
| `src/server/src/scene/mod.rs` `compute_derived` | M23b, M25 | New channel arms after `"combat"`. |
| `src/server/Cargo.toml` | M23, M27, M28 | Each dependency with its license comment (existing convention); alphabetical is NOT required — append under a `# Phase 3: <milestone>` comment. |
| `pnpm-lock.yaml`, root `package.json` | M22, M23, M24, M26 | Regenerate with `pnpm install`; on conflict take theirs then re-run `pnpm install --frozen-lockfile=false` and commit the result. |
| `src/client/render/src/engine.ts` | M22, M24, M25 | M22 owns `start()`'s ticker + `RenderEngineOpts.performance`; M24 adds `vfxView` reconcile as ONE call site inside the existing store-subscribe reconcile block; M25 adds the level filter through `scene-scope.ts`, not inside `engine.ts`. |
| `src/client/render/src/pixi-backend.ts` | M22, M24 | M22 owns `createPixiBackend` options + `resize`; M24 adds `setVfx`/`removeVfx`/`tickVfx` as new methods. |
| `src/client/render/src/backend.ts` (`DisplayBackend`) + `backend.mock.ts` | M22, M24 | Append new members at the end; the mock records structurally. |
| `docs/site/.vitepress/config.mts`, `docs/site/modules/*.md`, `docs/site/protocol.md` | all | New pages per milestone; sidebar entries appended to the relevant group. |
| `docs/PLAN.md`, `docs/HISTORY.md` | all | Only the LAST milestone to merge flips the phase heading; each milestone appends its own HISTORY entry under "## Phase 3 — Atmosphere" (create the heading if absent). |
| `~/.claude/skills/shadowcat-codebase/hooks/codebase-skill-reminder.py` `SUBSYSTEMS`, `skills/*` | all | Skill edits are made in the plugin checkout WITHOUT committing there; the dispatcher reviews and commits per milestone (§6). |

## 4. Global constraints (every milestone plan inherits these verbatim)

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
- **Binary size:** `pnpm lint:binary-size` guards the 60 MiB release binary; M28's runtime
  choice (§7) is made under it.
- **Server by default:** computation runs on the server; client-side work needs a reason
  (presentation, input capture, optimistic prediction). `docs/design/ARCHITECTURE.md` §2
  invariants 1, 6 and 11 govern every design fork.
- **UX outranks data secrecy** (invariant 11): send-then-hide is acceptable; PII and
  remote-device security are the two ironclad exceptions.
- `pnpm build` precedes any cargo build (rust-embed validates `dist/` at compile time).
- The Playwright suite is DISPATCHER-run on port 31999 (one suite at a time on the machine);
  a milestone WRITES its specs and runs unit/integration tests itself.

The full gate battery a milestone must show green before its PR (copy from M20's HISTORY
entry): `cargo test --all`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
`cargo clippy -- -D missing-docs -D clippy::missing-docs-in-private-items` (M27 adds this to
the three-OS matrix so `cfg`-gated modules are covered), `git diff
--exit-code src/types/generated` after regen, `pnpm -r typecheck`, `pnpm -r test`, `pnpm
build`, `pnpm lint`, `lint:docs`, `lint:props`, `lint:comments`, `lint:allowances`,
`lint:file-size`, `lint:inline-tests`, `lint:aria-labels`, `lint:gate-manifest`,
`lint:settings-privacy`, `lint:binary-size` (release build), `pnpm docs:check-examples`,
`pnpm docs:check-rust-examples`, `pnpm run test:scripts`, `pnpm run check:svelte-runtime`,
`pnpm --filter "shadowcat-example-*" build`, `pnpm --filter @shadowcat/core test:e2e`, and
`pnpm gate:push` (tree-keyed receipt) immediately before `git push`.

## 5. Integration order and the merge-forward protocol

Merge order into `main` (each via PR after BOTH CI runs are green; `main` is branch-protected,
`--auto` is off):

1. **M22** (small; every other milestone consumes `PerformanceSettings`).
2. **M28** (server-only; no seam consumers; lands early so its `Cargo.toml` hunk is the first
   conflict others resolve).
3. **M24** (defines the VFX seam M25 consumes).
4. **M23** (defines `AudioApi`; its `Cargo.toml` audio hunk lands after M28's).
5. **M25** (consumes M24's seam; touches `compute_derived` after M23b's arm exists).
6. **M26** (consumes only `PerformanceSettings`; last of the render-layer milestones so its
   overlay canvas sits on the final stage markup).
7. **M27** (consumes `AudioApi.duck`; its final task is the mixer wiring).

**Merge-forward protocol.** Every milestone's plan ends with an *integration task*: merge
`origin/main` into the branch (a merge commit, never a rebase — immutable history), resolve
conflicts by the §3 conventions, wire up every §2 seam it consumes, re-run the full battery,
then buddy-check the whole branch diff (spec + code reviewer, blind, diff pre-generated by the
dispatcher). A milestone whose consumed seam has not merged yet runs every other task first and
holds its integration task; it never stubs the seam. Milestone-internal tasks that depend on no
foreign seam start immediately in parallel.

**Dispatcher sequencing check before each PR:** `git rev-parse origin/main main` measured, the
branch contains `origin/main`, `pnpm gate:push` receipt present for the branch HEAD.

## 6. Skill-update gate per milestone

New `shadowcat-codebase-<subsystem>` skills (fixed shape: Purpose / Key files & seams / Hard
invariants / Gotchas / Pointers), created in the plugin checkout at
`~/.claude/skills/shadowcat-codebase/skills/`, each with its path globs added to
`hooks/codebase-skill-reminder.py`'s `SUBSYSTEMS` and one absolute-path assertion in the hook's
self-test (`bash hooks/test-codebase-skill-reminder.sh`, run FROM the plugin directory):

| Milestone | New skill | Existing skills updated |
|---|---|---|
| M22 | `shadowcat-codebase-performance` (`src/client/core/src/performance.ts`, `src/client/ui-kit/src/performance.svelte.ts`, `src/modules/settings/src/PerformanceEditor.svelte`, `src/modules/statusbar/src/PerfStats.svelte`, the ticker/resolution seams of `src/client/render/src/{engine,pixi-backend}.ts`) | `client-shell`, `scene-rendering` |
| M23 | `shadowcat-codebase-audio` (`src/client/audio/`, `src/modules/audio/`, `src/server/src/audio/`, `src/server/src/data/engine/audio*.rs`, the `"audibility"` arm) | `assets`, `scene-rendering`, `documents-permissions`, `realtime-sync` |
| M24 | `shadowcat-codebase-vfx` (`src/client/render/src/vfx-view.ts`, `src/modules/vfx/`, the `PlayVfx` frames) | `scene-rendering`, `actors-tokens`, `realtime-sync` |
| M25 | — | `scene-rendering` (levels, portals, `level_of`, explored keyed by level), `documents-permissions` |
| M26 | `shadowcat-codebase-dice-3d` (`src/modules/dice-3d/`, `DieRecord.kind`) | `dice`, `chat` |
| M27 | — | `audio` (duck sources), `server-ops` (the `audio-monitor` subcommand) |
| M28 | `shadowcat-codebase-sandbox` (`src/server/src/sandbox/`, the `validators` manifest key) | `module-toolchain`, `documents-permissions` |

Each milestone's coder edits the plugin checkout WITHOUT committing there; the dispatcher
dispatches `shadowcat-codebase:shadowcat-spec-reviewer` on the skill diff, runs
`node scripts/check-skill-symbol-refs-cli.mjs` (0 broken), `pnpm run test:scripts`, and
`node scripts/check-skill-api-refs-cli.mjs` (needs `pnpm build:all`'s `dist-docs`), then
commits + pushes in the plugin repo. `plugin.json`'s `version` is bumped ONCE at campaign end
(`1.6.0` → `1.7.0`).

## 7. Dependency and licensing decisions (settled here; milestone specs cite them)

| Need | Choice | License | Alternative rejected | Why |
|---|---|---|---|---|
| Audio decode (M23) | `symphonia` (pure Rust: mp3, flac, wav, ogg/vorbis, aac-lc, isomp4) | MPL-2.0 | FFmpeg (rejected in ARCHITECTURE §5) | Pure Rust, no C toolchain, covers every upload format a GM has. |
| Audio encode (M23) | `opus` crate over `audiopus_sys` (libopus, BSD-3, built by `cmake` on every runner — the `webp`/`cc` precedent) | MIT / BSD-3 | `vorbis_rs` (libvorbis) | Opus is the canonical derivative: half the bitrate at equal quality, decodable by every target browser in a WebM/Ogg container. The retained original is the fallback the client picks through `canPlayType` (M23 §4 pins the probe). |
| Ogg container (M23) | `ogg` crate | BSD-3 | hand-rolled | Tiny, maintained, pure Rust. |
| Web Audio (M23) | native `AudioContext` — NO `standardized-audio-context` shim | — | the shim ARCHITECTURE §4 named | Every supported browser (Chrome, Firefox, Safari ≥ 14.1, Android WebView, iOS WebKit) ships an unprefixed `AudioContext`; the shim adds a dependency with no remaining purpose. ARCHITECTURE §4's row is updated by M23. |
| 3D rendering (M26) | `three` | MIT | reusing the PixiJS WebGL context | PixiJS v8 is 2D-only and two renderers sharing one GL context fight over state; a separate transparent overlay canvas costs one extra context, which `PerformanceSettings.dice3d` turns off on constrained devices (decision D5). |
| Physics (M26) | `@dimforge/rapier3d-compat` (WASM, lazy-imported only when a roll plays) | Apache-2.0 | `cannon-es` (MIT, maintenance mode) | Deterministic, actively maintained, ~2 MB loaded only on first 3D roll. |
| WASM runtime (M28) | `wasmi` (pure-Rust interpreter, fuel metering, memory limits) | MIT / Apache-2.0 | `wasmtime` (Cranelift JIT, +15–20 MiB) | Validators run once per write over kilobytes; an interpreter is fast enough, has no JIT attack surface, adds ~1 MiB to a 60 MiB-capped binary, and builds on every matrix target without a C toolchain. |
| OS audio-session monitor (M27) | Windows: `windows` crate (WASAPI `IAudioSessionManager2`/`IAudioMeterInformation`); macOS: `coreaudio-rs` + `core-foundation` (process-tap via `AudioHardwareCreateProcessTap` on macOS 14.2+; older macOS reports "unsupported" honestly); Linux: `pipewire` crate (libpipewire, MIT; needs `libpipewire-0.3-dev` on the Ubuntu runner — a NEW `apt-get` CI step, the tree's first system-library dependency) | MIT / Apache-2.0 / MIT (stated from the crates' published metadata; M27's first task MEASURES them from the resolved `Cargo.lock` and records the result) | Discord Game SDK (proprietary, rejected) | The dependency/licensing review PLAN.md required is this row; M27 §2 records the per-platform capability matrix its first task measures. |

## 8. Campaign deliverable (the dispatcher's exit condition, measured not asserted)

- All seven branches merged to `origin/main` via PRs; `git rev-parse origin/main main` equal in
  `C:/Dev/Shadowcat`; CI green on `main`.
- `docs/PLAN.md`'s Phase 3 section reads "✅" with the M22–M28 pointer paragraph in the Phase-1/2
  style; `docs/HISTORY.md` carries one entry per milestone; `docs/OPEN_BUGS.md` and
  `docs/TODO.md` reflect measured reality (no new deferrals without the user's express
  authorization).
- ARCHITECTURE.md §3 gained the Phase-3 rows (symphonia/opus/three/rapier/wasmi/audio-monitor
  backends) and §4's audio / 3D-dice / ducking / sandbox / VFX / multi-level rows are struck or
  rewritten as built.
- The plugin repo holds every skill edit, committed and pushed, `plugin.json` at `1.7.0`.
- Every other branch and worktree deleted (local + origin), measured with `git worktree list`
  and `git branch -a`.

## 9. Decision log (design forks resolved under the best-long-term-shape rule)

- **D1 — Performance settings are per-DEVICE, in `localStorage`, not in the server-side
  `ui_state`.** A user on a phone and a desktop wants different budgets; `ui_state` is
  per-account and would drag the phone's cap onto the desktop. The theme mirror is the one
  existing `localStorage` use and its `readThemeMirror`/`writeThemeMirror` shape is the
  precedent (`sessionState.svelte.ts`). "auto" is resolved from device signals on each load.
- **D2 — Now-playing audio state is a server document, not an aux frame.** Joiners and
  reconnects must hear what the table hears; a document (`audio-state`) with a server-stamped
  `started_at` plus the existing `TimePing`/`TimePong` offset gives every client the same
  position without a replay protocol. Transport control (play/pause/seek/skip) is an intent on
  that document; a transient cue (a one-shot sfx) is an aux frame.
- **D3 — Audibility (spatial attenuation + wall occlusion) is computed on the server as a
  derived channel, per recipient.** Invariant 6's "server by default" and the existing
  elevation-banded raycaster (`scene::vision`) make the server the one place that already knows
  walls, elevation and the listener's tokens. The client applies gains; it never re-derives
  geometry (never-fork). Under invariant 11 the emitter documents themselves still reach the
  client (an inaudible emitter is muted, not stripped).
- **D4 — Levels are elevation bands on the scene, not separate scene documents.** Walls, lights
  and tokens already carry elevation and the raycaster already bands on it; a level is a named
  `[bottom, top]` band with its own background. Cross-scene travel is a portal's job, not a
  level's. Explored-fog memory becomes keyed by (scene, level, user) because a floor plan hides
  the one beneath it.
- **D5 — 3D dice render in a separate three.js WebGL context on a transparent overlay canvas
  above the stage.** See §7; the overlay is a module (`dice-3d`) contributing a surface, so a
  replacement UI can drop it, and `PerformanceSettings.dice3d` (default off on the mobile
  preset) removes the second context entirely.
- **D6 — The 3D result always matches the server's roll.** The client simulates locally (seeded
  from the roll id) and remaps each die's face textures after settling so the up-face shows the
  server's `DieRecord.value`; no reroll loops, no result the server did not author.
- **D7 — Ducking has three sources behind one contract.** The mixer's `DuckController` (M23)
  takes the max demand of any registered `DuckSource`; sources are: a mic voice-activity
  detector in the browser (works on every platform including mobile, no native code), the OS
  audio-session monitor (a `shadowcat audio-monitor` SUBCOMMAND of the single binary — never a
  second executable — serving a localhost WebSocket the client's source connects to), and a
  manual push-to-duck key. "Discord" is one process name in the monitor's default watch list,
  never an SDK.
- **D8 — Third-party validators run in `wasmi`, opted in per world by the GM, over the `system`
  band only, after the engine's own validation, with fuel + memory + instance-count caps and a
  per-call wall-clock deadline.** A validator can REFUSE a write (structured reason) and nothing
  else — it cannot mutate, read other documents, or reach the network. Failure of the sandbox
  (trap, out-of-fuel, malformed module) refuses the write and notifies the GM; the world keeps
  working with the validator auto-disabled after N consecutive faults (the sandbox is never a
  denial-of-service lever against the table).
- **D9 — Audio transcode produces ONE Opus derivative SIBLING and keeps the uploaded original
  as the CANONICAL, member-readable file.** This deliberately does NOT mirror the image
  pipeline's canonical swap (converted bytes canonical, original moved to the GM-only
  `/original` route): the audio original is every player's playback fallback, so it must stay
  on the normal serve route, and Opus rides `?variant=opus` like `.thumb.webp` does. The
  client chooses through `canPlayType` at load time; a missing derivative answers 404 and the
  client falls back — no lazy regeneration for a minutes-long transcode.
- **D10 — VFX assets are the two formats the pipeline already stores**: animated WebP (M15a
  stores animations pass-through) and a PNG/WebP spritesheet with a sidecar JSON (`asset` tag
  `vfx:sheet`). No new codec; a VFX "library" is an asset folder.

## 10. Verification of the campaign artifacts (this document and its siblings)

Each milestone spec is reviewed by a sonnet `shadowcat-codebase:shadowcat-spec-reviewer` before
its plan is written; each plan is checked against its spec by a second sonnet spec-reviewer
before its worktree is dispatched. Review findings are applied by the dispatcher (not by the
reviewer); a finding the dispatcher disagrees with is a design question for the user.
