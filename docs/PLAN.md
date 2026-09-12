# shadowcat — Roadmap

What remains to build, in order. MVP-first: Phase 1 (the dogfood alpha) is complete; later phases
add table features, atmosphere, then platform/scale. Each entry lists its goal, key deliverables,
and explicit exclusions. Architecture and rationale live in
[`design/ARCHITECTURE.md`](design/ARCHITECTURE.md). Completed milestones, with their delivery notes,
live in [`HISTORY.md`](HISTORY.md) — this file records nothing that has shipped.

Guiding rule: build what you cannot build on top of. Networking and permissions precede features;
features precede polish; the module API stays 0.x until evidence proves it.

## Phase 1 — MVP (→ dogfood alpha) ✅

Complete: M1–M13, the Phase-1 cleanup burndown, the close-out campaign, Phase 1b replay
redaction, all Bucket C follow-on sub-projects, and the debt-burndown campaign. Full record in
[`HISTORY.md`](HISTORY.md). `docs/OPEN_BUGS.md` is empty; `docs/TODO.md` holds only items blocked
on unbuilt Phase-2+ infrastructure or on external circumstances.

## Phase 2 — Full table ✅

Milestones in build order; each gets its own brainstorm → spec → plan cycle and may decompose
further at design time. Numbering continues from Phase 1.

### M14 · Combat tracker ✅
Complete: M14a (document/permission substrate), M14b (combat clock), M14c's six server-authority
sub-projects (server formula engine + invariant 6, combat resolution server-side, world-config
authority, dice references + chat channel, templates merge server-side, combat client seams) and
M14d (tracker module + settings editors) — delivery notes in [`HISTORY.md`](HISTORY.md)'s M14
entries. Automation of attack/damage resolution stays system-owned and audio/VFX cues stay
Phase 3, both by design.

### M15 · Asset pipeline + browser ✅
Complete: M15a (pipeline) and M15b (browser module + the generic GM-only document `Move`
operation) — delivery notes in [`HISTORY.md`](HISTORY.md)'s M15a/M15b entries. The FTS
integration for asset search shipped in M21 (see `HISTORY.md`'s M21 entry).

### M16 · Layout + theming completion ✅
Complete: M16a (theme engine — token data, controller, ui-state + pre-login persistence,
picker, dockview chrome, stage recolor), M16b (floating-window arrangement persistence and
gesture restore, keyboard move/resize, a11y resize targets), and M16c (custom theme editor
with live preview and contrast warnings, module styling modes, external-module stylesheets)
— delivery notes in [`HISTORY.md`](HISTORY.md)'s M16 entry.

### M17 · Vision, lighting + movement completion ✅
Complete: M17a (photometric field, carried emitters, light/wall authoring), M17b (vision-mode
descriptor v2, tremorsense + the perceived channel, elevation), M17c (movement-type tags +
terrain exemptions) and M17d (moving light source mid-walk: the carried-light timeline on
`MoveStream`, per-recipient reach admission, the client lighting sweep) — delivery notes in
[`HISTORY.md`](HISTORY.md)'s M17 entries. Web-Worker optimistic vision stays excluded (vision is
server-authoritative by design).

### M18 · Token enrichment ✅
Complete: generated token visuals, trigger regions, the aura/sound/VFX emitter component model,
per-token built-in fx (condition-driven + selection highlight), emote overlays, and token art
tooling — delivery notes in [`HISTORY.md`](HISTORY.md)'s M18 entry. Sound/VFX PLAYBACK remains
Phase 3 by design (the component model landed here; the emit seams are Phase-3 audio/VFX).

## Phase 3 — Atmosphere

Seven milestones, designed together (master integration spec:
`superpowers/specs/2026-09-11-phase3-master-integration-design.md` — seam ownership, shared-file
conventions, merge order M22 → M28 → M24 → M23 → M25 → M26 → M27) and built simultaneously in
separate worktrees. Each has its own design spec and implementation plan under
`superpowers/specs/2026-09-11-m2X-*-design.md` / `superpowers/plans/2026-09-11-m2X-*.md`.

### M22 · Performance settings + render budget
Per-device presets (auto / mobile / balanced / quality / custom): frame-rate cap, render scale,
antialias, token fx, lighting quality, VFX / 3D-dice / spatial-audio switches, dirty-flag idle
rendering, `prefers-reduced-motion`; a frame-stats readout. The seam every other Phase-3
milestone reads its budget from.

### M23 · Audio
M23a: Web Audio mixer (channels, per-device gains, duck bus), `playlist` + server-owned
`audio-state` documents with world-clock sync (joiners hear the table), scene ambience, the
Opus transcode derivative (`symphonia` + `opus`, original retained as the fallback), audio
panel + playlist sheet. M23b: per-recipient `"audibility"` derived channel (distance falloff +
elevation-banded wall occlusion computed on the server), spatial emitter playback.

### M24 · VFX
Server-derived grid sheets from animated WebP, PixiJS spritesheet pairing, the `vfx` render
layer with `VfxView` (token emitters + concurrent one-shots), `PlayVfx`/`Vfx` frames, the FX
scene tool through a new `SCENE_TOOL_CONTRACT`, the `/fx` chat command.

### M25 · Multi-level maps + portals
`SceneEngine.levels` (elevation bands with their own backgrounds), `ElevationBand` on walls /
regions / drawings / templates, movement + lighting + explored fog per level, client level
scoping + switcher, `TriggerEffect::Teleport` (same-scene and cross-scene portals through the
server-authored `Move`).

### M26 · 3D dice
`DieRecord.kind` exposed to every recipient; a `dice-3d` module rendering rolls in a separate
three.js WebGL overlay (rapier physics, seeded per roll, faces remapped to the server's values)
through a new `STAGE_OVERLAY_CONTRACT`; off by default on the mobile preset.

### M27 · Voice ducking
Three `DuckSource`s behind M23's contract: an in-browser mic voice-activity detector (audio
never leaves the worklet), a push-to-duck key, and the `shadowcat audio-monitor` subcommand
(WASAPI / Core Audio process tap / PipeWire) serving watched-process levels over a localhost,
origin-allowlisted WebSocket; a new `SETTINGS_SECTION_CONTRACT`.

### M28 · Sandboxed third-party validators
The parked capability Phase 3: opt-in, per-world `wasmi` validators declared by a module's
manifest, run over the `system` band outside the write transaction with fuel / memory /
wall-clock caps, refusal reasons on a new `Reject.detail` field, auto-disable on faults, a
threat model and an example validator crate.

## Phase 4 — Platform & scale
**Audit-grade point-in-time replay** — a state-as-of-sequence facility: what a document, its
permissions, its effective owner and the world's capability grants were at any past sequence, so a
replayed event can be redacted against the policy that actually applied. Phase 1b established the
prerequisite: a commit-time redaction snapshot carried on every operation
(`StoredCommand`/`CommandSnapshot`/`OpSnapshot`), against which `filter_command` redacts in
conjunction with current state. This milestone generalizes that into a queryable history. **When it
lands it must become the single producer feeding the existing redaction-context interface — a
second, independently-derived source for the same decision is the fork-a-decision class this
codebase produces most.**
→ Trusted local modding hardening → freeze the module API on evidence (≥1 external module ships without core patches, **or N internal modules across M independent systems exercise the full API surface** — whichever comes first, so the freeze is not deadlocked on an external author who may never appear) → [only if a marketplace is pursued] WASM sandbox + registry + signing / SRI / CSP + package browser → native wrappers (Tauri 2, Capacitor) → hardening + distribution (backup scheduling / automation, world snapshots, WS load + resync stress tests, rate limiting, rustls-acme TLS, Steam OpenID + plain-executable distribution).
## Documentation campaign (cross-phase, runs alongside feature work)

Infrastructure, guides, and Sweeps 1–14 are complete (record in [`HISTORY.md`](HISTORY.md)).
Remaining, in order:
- **Buddy-check convergence — after the last sweep (user directive 2026-07-30).** The completed
  first-pass documentation is buddy-checked (superpowers two-reviewer cross-check debate)
  **crate by crate**: the `shadowcat` server crate, then each TS workspace package. Any problems
  surfaced → fix → re-buddy-check THAT crate; repeat until a buddy-check pass finds no problems.
  Only then does the final ratchet run. Required reading for every implementer and reviewer:
  `docs/design/doc-sweep-truthfulness-rules.md`.
- **Final ratchet — after buddy-check convergence.** Crate-root deny attributes (`lib.rs` is the
  one file in the server crate still without them, reserved for this step), TypeDoc
  `treatValidationWarningsAsErrors: true` (in `typedoc.base.json`), docs lint merged into the main
  `eslint.config.js`.
- **Skills documentation-reference pass — after the final ratchet (user directive 2026-07-30).**
  Every `shadowcat-codebase-*` skill's Pointers section gains its documentation references
  (subsystem rustdoc path under `/api/rust/`, TypeDoc package pages under `/api/ts/`, relevant
  guide/protocol/module portal pages), via the reviewed skill-update gate.

## Cross-cutting (not deferred)
- Data migrations: NONE are built pre-customers — `migrations/0001_init.sql` is a single
  baseline edited in place, and `data/migrate.rs` stays step-free machinery. This line is the
  campaign marker: when a release milestone declares live customer databases, flip this entry
  and start authoring real incremental migrations from that point on.
- Desync-convergence test (M4): maintained throughout.
- Backups: the basic backup + snapshot-restore deliverable (M12.5) satisfies the dogfood gate;
  Phase 4 adds scheduling / automation.
- Rate limiting on WS / upload: introduced with the surfaces it protects, not only at hardening.
- Account model: self-host, admin-provisioned accounts (admin-only `POST`/`GET /api/users`) plus a
  GM-minted world invite the invitee redeems from their own session. Deliberately no
  self-registration / email in v1; the invite exists so a GM never has to name a user, which is
  what keeps username existence secret.
