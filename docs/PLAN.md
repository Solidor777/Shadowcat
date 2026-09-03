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

## Phase 2 — Full table

Milestones in build order; each gets its own brainstorm → spec → plan cycle and may decompose
further at design time. Numbering continues from Phase 1.

### M14 · Combat tracker
- **M14a (document/permission substrate) and M14b (combat clock) are DONE** — full delivery
  notes in [`HISTORY.md`](HISTORY.md)'s M14a and M14b entries.
- **M14c — server authority + combat client seams**, six sub-projects in build order (design:
  [`superpowers/specs/2026-08-30-m14c-1-server-formula-engine-design.md`](superpowers/specs/2026-08-30-m14c-1-server-formula-engine-design.md) §1):
  - **M14c-1 — server formula engine + invariant 6** — DONE (see [`HISTORY.md`](HISTORY.md)).
  - **M14c-2 — combat resolution server-side** — DONE (see [`HISTORY.md`](HISTORY.md)).
  - **M14c-3 — world-config authority** — DONE (see [`HISTORY.md`](HISTORY.md)).
  - **M14c-4 — dice references + chat channel** — DONE (see [`HISTORY.md`](HISTORY.md)).
  - **M14c-5 — templates merge server-side**: `MergePull`/`MergePush`/`MergeRevert` intents;
    conflict set returned for human review; `Document.base` under engine-tree validation.
  - **M14c-6 — combat client seams**: `AppContext.combat`, `CoreHooks` first entries +
    delta-derived emission, `Warn` overage label.
- **M14d — tracker module + settings editors** (panel + settings-editor conventions to follow
  the M15b asset-browser module, which landed first): the default tracker UI, the world/scene combat
  settings editors (including the combat chain editor over `resolve_combat_rules`'s
  engine→system-defaults→world→scene precedence), and end-to-end coverage.
- Depends on: M11 dice, the M10 movement executor, M14a+M14b (done).
- Excludes: automation of attacks/damage resolution (system-owned); audio/VFX cues (Phase 3).

### M15 · Asset pipeline + browser ✅
Complete: M15a (pipeline) and M15b (browser module + the generic GM-only document `Move`
operation) — delivery notes in [`HISTORY.md`](HISTORY.md)'s M15a/M15b entries. The FTS
integration for asset search remains deferred to M21 by design.

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

### M19 · Tables, notes + chat media
- Rollable tables on the dice engine + document model (weighted rows, nested draws, results to
  chat as roll embeds).
- Rich-text notes on the document model (journal-style documents; reuse the chat sanitizer
  boundary and `Segment::DocLink` for cross-references).
- Chat media linking: images; YouTube as thumbnail + external link only — no IFrame / Data API.

### M20 · Full default module suite
- Every table-facing default module the dogfood alpha lacks, shipped as `src/modules/*` packages
  over the M14–M19 seams (combat tracker UI, asset browser UI, table/notes sheets, emitter
  editors), each independently replaceable.
- The suite is the second internal-module exercise of the API surface toward the Phase-4 freeze
  gate.

### M21 · Search consolidation
- One search milestone, one backend: extend the M6c FTS5 index + live subscriptions to every
  document type the suite introduces (notes, tables, assets by tag) with the same
  visibility-partitioned index — no three-backend split.
- Folds M15's asset search into FTS: M15 deliberately ships `GET /api/assets` as SQL
  substring/tag/folder filters plus a size-limited `regex` name filter (asset names are short;
  a `LIKE` + tag join suffices) — the FTS integration was deferred here, not forgotten.

## Phase 3 — Atmosphere
Audio (mixer, channels, playlists, world-clock sync; then spatial + wall occlusion; transcode via `symphonia` + `opus`/`vorbis_rs`) → VFX (sprite effects, concurrent SFX) → multi-level maps + portals → 3D dice (decide the rendering context up front: reuse the PixiJS WebGL context vs a separate three.js/WebGL + physics layer) → Discord audio-ducking module (OS audio-session monitoring — PipeWire / WASAPI / CoreAudio — never the proprietary Discord Game SDK; requires a dependency / licensing review before integration).

Also parked for Phase 3 from Phase 1: capability Phase 3 — opt-in **sandboxed** server-side
validators running third-party *code* (its own threat model; never the default path). Server-side
evaluation of the engine's own grammars is not this item — it shipped in M14c-1.

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

Infrastructure, guides, and Sweeps 1–13 are complete (record in [`HISTORY.md`](HISTORY.md)).
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
