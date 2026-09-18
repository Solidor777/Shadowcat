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

## Phase 3 — Atmosphere ✅

Complete: M22–M28, designed together (master integration spec:
`superpowers/specs/2026-09-11-phase3-master-integration-design.md`) and built simultaneously in
separate worktrees, merged in order M22 → M28 → M24 → M23 → M25 → M26 → M27 — delivery notes in
[`HISTORY.md`](HISTORY.md)'s M22–M28 entries.

### M22 · Performance settings + render budget ✅
Complete: per-device presets (auto / mobile / balanced / quality / custom) and the frame-stats
readout — the seam every other Phase-3 milestone reads its budget from — delivery notes in
[`HISTORY.md`](HISTORY.md)'s M22 entry.

### M23 · Audio ✅
Complete: M23a (Web Audio mixer, `playlist` + `audio-state` documents, scene ambience, Opus
transcode, audio panel + playlist sheet) and M23b (per-recipient `"audibility"` derived channel,
spatial emitter playback) — delivery notes in [`HISTORY.md`](HISTORY.md)'s M23 entry.

### M24 · VFX ✅
Complete: server-derived grid sheets, the `vfx` render layer, `PlayVfx`/`Vfx` frames, the FX
scene tool through `SCENE_TOOL_CONTRACT`, the `/fx` chat command — delivery notes in
[`HISTORY.md`](HISTORY.md)'s M24 entry.

### M25 · Multi-level maps + portals ✅
Complete: `SceneEngine.levels`, `ElevationBand` on walls / regions / drawings / templates,
per-level movement/lighting/fog, client level scoping + switcher, `TriggerEffect::Teleport` —
delivery notes in [`HISTORY.md`](HISTORY.md)'s M25 entry.

### M26 · 3D dice ✅
Complete: `DieRecord.kind` exposed to every recipient, the `dice-3d` module's three.js/rapier
overlay through `STAGE_OVERLAY_CONTRACT`, off by default on the mobile preset — delivery notes
in [`HISTORY.md`](HISTORY.md)'s M26 entry.

### M27 · Voice ducking ✅
Complete: the three `DuckSource`s behind M23's contract (in-browser VAD, push-to-duck,
`shadowcat audio-monitor` subcommand) and `SETTINGS_SECTION_CONTRACT` — delivery notes in
[`HISTORY.md`](HISTORY.md)'s M27 entry.

### M28 · Sandboxed third-party validators ✅
Complete: opt-in per-world `wasmi` validators over the `system` band with fuel/memory/wall-clock
caps, `Reject.detail`, auto-disable on faults, threat model + example validator crate —
delivery notes in [`HISTORY.md`](HISTORY.md)'s M28 entry.

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
