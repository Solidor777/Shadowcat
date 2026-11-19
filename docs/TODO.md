# TODO — Deferred Work

Actionable, externally-logged deferrals. Bugs go in `OPEN_BUGS.md`, not here.
As of the Phase-1 cleanup burndown (2026-07-19), most items below are
retained because their blocking capability doesn't exist yet — a concrete
unblocking condition, not a "someday maybe." A few headings are explicitly
labeled "Actionable now": these are NOT blocked on anything — the underlying
capability already exists — but are deferred as out-of-scope-for-now work.

## Blocked on release-profile records of the send-buffer gate on Linux and macOS
- TODO: Make the send-buffer gate's `buffer_ratio >= 0.90` assertion blocking in release builds.
  The gate (`h2_send_buf_gate`) records `buffer_ratio` (bare hyper at `H2_SEND_BUF_BYTES` against
  bare hyper at the library default) and `production_ratio` per run, to stdout and the CI step
  summary, and asserts only that every download delivers every byte. The 0.90 figure is empirical
  (a throughput ratio on shared hardware has no derivation), so the numeric-assertion rule keeps
  it record-only until every operating system's value is recorded. Trigger: one CI run in which the
  "Send-buffer throughput gate (release, record-only)" step has printed `profile=release` records
  on ubuntu, macos and windows. Records must come from the current harness (the production send buffer and
  TCP_NODELAY set on the accepted sockets, as the production listeners do); Windows loopback in release read `buffer_ratio` 0.958,
  0.956 and 0.954 with it (4 workers, 100 downloads, 3 runs). Two instruments read the production
  path against bare hyper at 64 KiB: `egress_measure layers` (2 workers by default) reads the production
  path 3.1% slower, and the gate reads it 4.9% to 6.2% slower. The owner ruled that production-path
  difference noise: there is no open reconciliation and nothing to rerun. The ruling does not cover
  the buffer-ratio difference: the gate reads `buffer_ratio` 0.954 to 0.958 and the ladder reads the
  same quantity (default-buffer median over 64 KiB median) 0.995 for the `bare-64k` rung
  (`bare-64k+full-router` prints 0.996). That difference is an unexplained observation. The figures
  below are the durable copy (per-run medians of per-download times); the records they were taken
  from live in the ignored, ephemeral debug dump directory. Regenerate them with `cargo run
  --release -p shadowcat-test-support --bin egress_measure -- layers <downloads> <runs> <workers>`
  and `cargo test --release -p shadowcat --test h2_send_buf_gate -- --nocapture`.
  Differences between the two instruments, none isolated; candidates, not causes:
  - Worker count: the gate runs 4 and `layers` defaults to 2 (it accepts a worker argument).
  - Host regime, which co-occurs with the shift but is confounded with run position. Within each
    of three 2-worker subset records of `layers` (3 runs, 100 downloads, three rungs), run 1 is
    in the fast regime (default-buffer 11.47 to 11.66 ms, 64 KiB 11.57 to 11.71 ms) and reads
    0.991 to 0.997, and runs 2 and 3 are in the slow regime and read 1.06 to 1.27, at identical
    workers, downloads and rungs. That is the one controlled observation, and run 1 is fast in
    all three files.
  - Downloads per run: gate 100; the fast-regime `layers` ladder record 150; the slow subset
    records 100.
  - Rung set: the fast-regime `layers` record runs the full 11-rung ladder; the slow records run
    three rungs (`bare-default-buf`, `bare-64k`, `serve_router+full-router+observed-source`).
  - Binary: in the fast regime the gate's 64 KiB leg reads 12.52 to 12.59 ms against 11.64 to
    11.74 ms in the `layers` ladder.

  Ranges are per-run medians, each named by instrument, workers and rung:
  - `layers`, 2 workers, `bare-default-buf`: fast 11.47 to 11.71 ms (150-download ladder 11.58 to
    11.71; subset run 1 11.47 to 11.66); slow 38.3 to 42.9 ms (subset runs 2 and 3).
  - `layers`, 2 workers, `bare-64k`: fast 11.57 to 11.74 ms (ladder 11.64 to 11.74; subset run 1
    11.57 to 11.71); slow 29.2 to 36.7 ms (subset runs 2 and 3, and a separate 8-run
    `bare-64k`-only record whose first two runs read 11.73 and 11.79 and the other six 29.24 to
    36.62).
  - `layers`, 4 workers, `bare-default-buf`: 25.0 to 54.9 ms over six 3-run, 100-download files
    (no fast-regime record exists at 4 workers); the three files taken last read 27.5 to 54.9.
  - `layers`, 4 workers, `bare-64k`: 16.4 to 41.7 ms over the same six files; the last three read
    20.7 to 41.7.
  - Gate (`h2_send_buf_gate`), 4 workers, default-buffer leg: fast 11.94 to 12.07 ms; slow 26.9 to
    49.6 ms (nine runs). Buffer leg: fast 12.52 to 12.59 ms; slow 26.1 to 41.8 ms.

  Readings:
  - 2-worker `layers` reads 0.995 in the fast-regime record, per file, over five runs (per run
    0.994 to 0.997; 150 downloads, full ladder), and 1.10 to 1.25 per file in the three
    slow-regime subset records (whole files, run 1 included).
  - The only equal-worker (4) `layers` readings were taken in the slow regime: default-buffer over
    64 KiB 1.21 to 1.30 per file (per-run 1.02 to 1.41, nine runs), while the gate in the same
    state read `buffer_ratio` 0.946 to 1.309 per run across nine runs. No fast-regime reading of
    the ladder at 4 workers exists (the gate already gives a fast 4-worker buffer ratio of 0.954 to
    0.958).
  - Six alternating pairs of fresh single-run processes of `layers` (three rungs each): 4 workers
    read 25.7 to 33.3 ms and 2 workers read 11.6 to 12.4 ms, so the 4-worker reading is slow even
    as run 1 of a fresh process and worker count is not cleanly separable from run position or
    regime.

  Owner ruled: measure the buffer-ratio gap later. Host observation, not a blocker and with no
  diagnosed cause: a 2-worker process of these servers runs about 11.6 ms per download in the
  fast regime and reads the slow ranges above in later runs of the same session. Then, in the same
  branch: assert `buffer_ratio >= FLOOR` when `cfg!(debug_assertions)` is false, with `FLOOR` set
  below the worst of the three legs by a stated headroom and the three recorded values written
  beside the constant. The debug profile stays record-only (its ratio is a profile artifact, 0.66
  to 0.79 measured) and `production_ratio` stays recorded.

## Blocked on verifying `--no-experimental-webstorage` against CI's pinned Node version
- TODO: `pnpm -r test` fails locally on Node 25.3.0+ with `TypeError: localStorage.clear is not
  a function` in any package whose tests call `localStorage` directly (e.g.
  `dice-3d`'s `settings.test.ts`) — Node's own native `localStorage`/`sessionStorage` globals
  (unflagged by default starting some Node 24/25 release) collide with jsdom's own
  implementation of the same globals. CI is unaffected (pinned to Node 24, which does not inject
  the native globals unflagged). The working per-invocation fix is
  `NODE_OPTIONS=--no-experimental-webstorage pnpm -r test` (or `gate:push`/`gate:commit`), but a
  permanent repo-tracked fix (root `.npmrc`'s `node-options`) risks breaking CI's Node 24 run if
  that flag isn't recognized there — unverified. Confirm compatibility with CI's exact Node 24.x
  patch version before adding it to a tracked config file; until then this stays a manual
  per-invocation env var on any local machine running a newer Node.

## Blocked on real pointer-gesture QA (unsimulable under jsdom)
- TODO: `DockviewEngine#toDropSite`'s one remaining fallback branch (a drop's target group
  falling outside the engine's own zone bookkeeping) is a best-effort approximation (falls back
  to an edge-zone dock), not exhaustively verified against every dockview drag path. The
  intercept-and-redispatch translation mechanism itself (preventDefault + emit + reconcile
  through `apply()`) IS exercised directly by unit tests now, not approximated. Real drag-and-drop
  still cannot be simulated under jsdom (no native `DragEvent`/`PointerEvent` gesture) — the
  residual manual-QA item narrows to drop-position classification fidelity for real pointer
  geometry (edge vs center vs tab-strip index resolution against an actual drag gesture) before
  shipping.

## Blocked on a bespoke-fallback caller needing it
- TODO: `FakeEngine`'s plain tab strip has no `PanelMenu` (dock/float/minimize/pop-out commands)
  — that menu is mounted by `DockviewEngine.createTabComponent` only. A panel docked under
  `FakeEngine` (bespoke-fallback engine; production never reaches it) can only reach a
  minimized/closed state going forward via `PanelsChipsView.restore`, not back out of a zone
  through any UI affordance. Orthogonal to the width-containment fix (`docs/CLOSED_BUGS.md`):
  giving `FakeEngine` its own menu is future work if a bespoke-fallback caller needs it.

## Actionable now — e2e context-seeding consistency
- TODO: `levels.spec.ts` and `audio.spec.ts` construct their player browser context via a bare
  `browser.newContext(...)` instead of the shared `newE2EContext` helper (`fixtures.ts`), which is
  the one place `seedPerformanceMirror` applies — every other dual-session spec uses it. Neither
  spec currently asserts on anything gated by `AppContext.performance` (levels checks scene/
  teleport geometry, audio checks server-authoritative playback state and role-gated UI), so this
  is not a live bug — CI is green on main with both as-is. It is a latent landmine: if either
  spec's scope grows to cover a performance-gated feature (dice3d, vfx, spatialAudio,
  reducedMotion, fpsCap) on the player side, it will silently read `hardwareConcurrency<=4` on
  CI's runners and fail the same way `dice-3d.spec.ts` did. Switch both to `newE2EContext` for
  consistency with the rest of the suite.

## Actionable now — Kimi Code parity is written but never installed
- TODO: The skill/agent source moved to the standalone `shadowcat-codebase` plugin repo
  (`github.com/Solidor777/shadowcat-codebase`); this item now targets
  `~/.claude/skills/shadowcat-codebase` (or a fresh clone of that repo), not `<Shadowcat
  checkout>/.claude`. In the Kimi TUI run `/plugins install ~/.claude/skills/shadowcat-codebase`
  then `/reload` (the
  third-party trust prompt defaults to cancel — approve it). CLI/prompt-mode install does not
  exist. Then confirm all six agents register: the twelve ported copies carry Claude-native
  `tools:` names including `Skill`, and whether Kimi recognizes them is unverified — if
  unrecognized entries reject the frontmatter, the agents will not load at all. Separately,
  their bodies say "invoke via the Skill tool", but Kimi's config sets
  `merge_all_available_skills = true`, suggesting skills may be merged into context rather than
  invoked; if so the bodies' BLOCKED branch could fire in a consumer repo even when the context
  is already present. Both need one real dispatch in a consumer workspace to settle.
  - **Attempted, genuinely blocked (not a deferral):** the `kimi` CLI is installed and its
    `config.toml` validates clean (`kimi doctor config`), but `kimi -p "..."` returns
    `provider.api_error: 403 You've reached your usage limit for this billing cycle` — the
    account's quota is exhausted, refreshing next billing cycle. This blocks any real dispatch
    through Kimi (TUI or `-p`/`--agent-file` non-interactive mode) regardless of plugin-install
    status, so the agent-registration and skill-invocation questions above remain unverified for
    a reason outside this session's control. Re-attempt once quota refreshes or extra usage is
    purchased.

## Blocked on owner approval — GM/player-facing improvements from the comparative VTT survey

Each item below would change what a GM or player sees or does, so it is NOT built until the
owner approves or rejects it case by case (owner directive: only invisible, uncontested
improvements ship without a ruling). Items are grouped by subsystem; a size tag is S/M/L; a
"contested" tag means a reasonable engineer could defend the current behaviour or the change
conflicts with a documented decision — the note says why. Items marked "design question" need
a ruling on shape, not just a yes/no. None is blocked on an unstarted `PLAN.md` milestone.

### Walls, regions, templates, movement (scene-rendering)
- TODO: Doors — `WallEngine` gains `door: Option<DoorState>` (`Closed`/`Open`/`Locked`); the
  effective `blocks_move`/`blocks_sight`/`blocks_light` become ONE resolver over the authored
  flags and the door state (never a second copy of the flags), and a narrow non-GM `Update`
  exemption may flip `Closed`↔`Open` only (never onto or off `Locked`), shaped like the existing
  narrow exemption `permission::carried_light_touched` classifies. Wall editor + click-to-toggle
  affordance in `ToolRail`. (M)
- TODO: One-way walls — a direction field on `WallEngine` tested by a cross-product side check
  inside `vision::visibility_polygon`'s ray-segment intersection and `move_exec`'s wall gate.
  Touches the fail-closed vision surface; needs its own security re-verification. (M)
- TODO: Wall-view render treatment for one-way walls — a directional tick/shading on
  `wall-view.ts` so a GM can see a wall's `blockingSide` orientation at a glance. Deferred out of
  the one-way walls work as pure client-render polish; the data model and editor controls landed
  without it. (S)
- TODO: Independent `blocks_sound` on `WallEngine` with a `sound_wall_entries` accessor;
  `scene::audibility::compute_audibility` currently reuses `sight_wall_entries`, so sound
  occlusion is aliased onto sight-blocking walls. Default for existing walls: inherit
  `blocks_sight` (no migration). (S, mildly contested — the aliasing is documented as
  deliberate)
- TODO: Tri-state authoring for `blocksSound` in the wall editor — the landed checkbox writes
  `Some(true)`/`Some(false)` but offers no path back to `null` (inherit `blocksSight`), so once
  touched the inheritance state is unrecoverable through the UI. A tri-state control or a
  clear affordance (the door `<select>`'s `none`-writes-`null` precedent) is the long-term
  shape. Surfaced by code review as owner-awareness, not a defect in the landed slice. (S)
- TODO: LIMITED partial-blocking tier per wall flag (see through one such wall, not two) — a
  per-ray crossing count in `vision::visibility_polygon`'s `nearest_hit`. (M-L, contested:
  polish; must land together with any other change to that kernel)
- TODO: Proximity/distance-threshold sense tiers on walls with optional attenuation. (L,
  contested: niche for the geometry cost across three raycast paths)
- TODO: Vertex/edge snapping for wall (and region corner) placement — `Grid.snap` is
  documented and implemented as CELL-CENTER ONLY; `makeWallTool` snaps both endpoints to
  centers, so walls cut through cell interiors instead of running along boundaries.
  `Grid.cellVertices` already exists to build a `snapVertex` mode on. (S-M, contested:
  center-only is a stated design decision in the code)
- TODO: Edge-snap variant for the snap-mode toggle — nearest point on the nearest cell EDGE,
  for wall mid-span placement. The vertex snap mode landed (`Grid.snapVertex`, GM-only
  `snap-mode-toggle`, default center); edge-snap was scoped out of it as an unresolved scope
  question (the vertex case covers wall endpoints and region corners, which is all the
  original item's body elaborated). (S)
- TODO: Diagonal rules — add `DiagonalRule::Approximate` (diagonals cost 1.5×) to
  `scene::pathfinding` and the `Grid` client mirror (S); a no-diagonal rule changes the
  pathfinder's ADJACENCY (`astar_leg`/`neighbors_with_cost`, `Grid.distance`), a separate
  design pass (M, mildly contested)
- TODO: `TriggerEvent::Exit` — regions fire `Enter`/`Arrest` only, so `ConditionAdd` on entry
  has no automatic `ConditionRemove` on leaving; fire it from the same per-step cell-transition
  detection `move_exec` already performs, through `regions::fired_triggers` /
  `Room::fire_region_triggers`. Needs persisted prior-tick token↔region membership. (S-M)
- TODO: Turn/round-scoped region triggers (`TurnStart`/`TurnEnd`/`RoundStart`/`RoundEnd`) fired
  from `combat::transition` against `SceneEcs::trigger_regions`' cell identity rows, with
  once-per-turn dedup. (M-L)
- TODO: Region-trigger failure diagnostics are authored by the mover/advancer, and
  `build_message_doc` grants a message's author Owner read — so a player whose move or turn-end
  fires a BROKEN trigger on a secret region learns that a trigger exists there (the GM-only
  audience is pierced by authorship). Shape: author the diagnostic as a GM/seed author, or make
  the failure notice's audience rule independent of the mover. Pre-existing for movement
  triggers; the turn/round slice widened its reach. (S, mildly contested: the diagnostic is
  genuinely useful to the acting player for non-secret regions)
- TODO: A public `chat_notice` on a public region fired at a HIDDEN combatant's turn/round
  boundary discloses by timing that a hidden combatant exists and crossed a boundary (including
  compressed turn-start+turn-end pairs for auto-resolved hidden combatants, whose turns never
  visibly park). Accept explicitly, or force GM-only when the firing combatant is hidden. (S,
  design question)
- TODO: Compound regions — `RegionEngine.shape` becomes a list of shapes each with a `hole`
  flag; `regions::rasterize` composes additive shapes minus holes. (M-L)
- TODO: Volumetric sense-restriction regions (silence zone, darkness globe, sight-blocking fog)
  — regions become occluders for `scene::vision`/`scene::lighting`/`scene::audibility`, which
  today read only the wall set. (L)
- TODO: Token-attached (following) regions — a mechanical mobile aura; every region consumer
  (`region_field`, `pathfinding`, `trigger_regions`) assumes static per-scene geometry today.
  (L, contested)
- TODO: Multi-destination portals with player choice/reveal flags — `TriggerEffect::Teleport`
  has one fixed `PortalTarget` and fires unconditionally in `Room::fire_region_triggers`; a
  choice needs a pausable move, i.e. a wire mechanism that does not exist. (M-L, contested)
- TODO: Arc-tessellated cone base so a template aperture of a half-turn or wider is
  representable — `conePoints` returns a three-point wedge, which is collinear at exactly 180° and
  opens away from `direction` beyond it, so `TemplateEngine::validate` bounds `shape.angle` to
  `(0.0, 180.0)`. Widening that bound requires tessellating the base as an arc, which changes the
  default cone's output and so needs its regression pin re-derived. (S-M)
- TODO: Translate enum `<option>` labels in `GameSettingsPanel` — the movement-restriction,
  movement-model, light-mode, diagonal-rule, easing and illumination-floor selects render their raw
  wire values as option text, so a GM reads identifiers rather than words. The same file already
  translates per value for the perception, dice-mode, dice-direction, audio-occlusion and fog-pooling
  selects, so the pattern exists; this is the set that predates it. (S)
- TODO: Scene padding margin around authored `SceneEngine.bounds`. (S, contested: the
  grid-units-first bounds model does not have the problem padding solves)
- DESIGN QUESTION: unify `TemplateEngine` and `RegionEngine` on one geometry/rasterization
  engine so a template can carry trigger effects — the two lifecycles (ephemeral author-owned
  vs persistent GM-authored elevation-banded) differ enough that a shared primitive may add
  complexity. (L)
- DESIGN QUESTION: a whole-session pause concept (only a combat-scoped pause exists).

### Vision and lighting
- TODO: Elevation-aware (3D) light/vision range — `visibility_compute::point_qualifies`,
  `lighting::source_level` and `cell_visible` use planar distance only; `scene::elevation`
  documents "ranges stay 2D". (M, contested — design question: discrete floors vs 3D falloff)
- TODO: Per-light darkness-activation window on `LightEmission`, checked against
  `settings.env_intensity` in `cell_illumination_from`. (S)
- TODO: Negative/darkness lights with priority suppression — inverts the monotonic-brightening
  invariant `visible_cells`/`player_lit_mask`/the movement gate/explored fog rely on; needs its
  own design + security pass. (L, strongly contested; build the region override first)
- TODO: Cone lights (`LightEmission` `angle`/`rotation`) — SPLIT from facing-limited player
  vision, which is a ruleset choice many tables reject. (L, the vision half contested)
- TODO: Structured vision-mode visual descriptor — the client's `lighting.ts` `resolve`
  interprets only the literal `render_hint` value `"desaturate"`; any other hint a GM authors
  in the `VisionModesEngine` editor silently produces no treatment. (M)
- TODO: A `TokenFx` marker for tokens revealed through a creature sense (the server's
  `perceived` list) so a player can tell which sense revealed them; client-only. (S)
- TODO: Condition-gated vision (blinded suppresses `gather_vision_sources_in_scene` floors;
  invisible excludes the target from ordinary sight admission). Blocked on conditions having
  mechanical effects at all (see combat section). (M)

### Combat, actors, effects, tokens
- TODO: Effect modifier pipeline — `EffectEngine` gains a `changes` list (key, mode, formula
  value, priority) folded into `SystemLeafResolver` reads at resolution time, never written
  back. (L, contested: could live entirely in system-module territory — design question)
- TODO: `EffectEngine.origin: Option<String>` provenance field — held because nothing reads it
  yet; add with its first consumer. (S)
- TODO: Combatant groups (shared initiative slot) — `CombatantGroupEngine` parented like
  `combat-history`, `CombatantEngine.group`, `transition::roll`/`sort`/`rebuild_order` treat
  same-group members as one initiative. (M)
- TODO: `CombatantEngine.defeated` + skip-defeated in `settle_turn`'s auto-resolve walk. (S-M)
- TODO: Per-token disposition override (friendly/neutral/hostile) independent of
  `Faction.stance`, and a secret-identity mode extending name-only `OwnerOrGm` redaction to the
  whole presented identity. (M, contested — two separate decisions)
- TODO: Token nameplates + attribute bars — `resolveTokenBarAttribute` beside
  `resolveTokenActor`, `TokenNodeSpec.bars`, display-mode gating (none/hover/owner/always). (L)
- TODO: Real-time effect durations need a world clock; `DurationUnit` is `Rounds`/`Turns` only.
  (L, contested — new subsystem)
- TODO: Scene-less/global combat — `CombatEngine.scene_id` is mandatory by `validate_containment`
  and is what keeps the visibility/movement-gate model simple. (L, contested)
- TODO: `Condition.overlay: bool` — render at most one active overlay condition large. (S)
- TODO: Anchor/scale/tint on plain image/animated `TokenVisual` kinds (only `"generated"` has
  crop/border/background). (S-M)
- TODO: Auto-rotate token to face movement + lock-rotation override; nothing drives `rotation`
  from movement today. (S, contested: divisive cosmetic)
- TODO: Token quick-edit (bars, elevation, conditions) anchored in the keep-mounted `ToolRail`
  rather than a zoom-scaled canvas HUD, sized independent of canvas zoom. (L, placement
  contested)

### Chat, dice, tables, notes
- TODO: Opaque `system` payload on `ClientMsg::SendMessage` persisted into the message
  `Document.system` (same structural caps as every other doc type) plus a card-extension
  contribution so a module renders against it without `MessageCard.svelte` knowing. Message is
  the one doc type a module cannot extend today. (L, needs API design)
- TODO: Per-die `min`/`max` clamp modifiers in the dice grammar (`dice::notation`). (S)
- TODO: Natural-min/max die highlight in `SegmentList` independent of `SuccessCount` crit
  config. (S)
- TODO: Pool keep-best-total across expressions. (L, contested: `label`/`compare_labels` covers
  the common advantage case)
- TODO: Parity (even/odd) success rule (S, contested); computed dice-group count (L, contested).
- TODO: Revise a message's `audience` after send (GM reveal/conceal). (S-M)
- TODO: Blind roll hidden from the roller's own client — conflicts with the documented
  "roller always sees their own roll" decision. (M, contested)
- TODO: Roll-mode/whisper chat shortcuts (`/gmroll`, `/publicroll`, `/gm`, `/reply`,
  `/players`, `/ooc`, `/ic`) in `chat::commands::parse_command`; `/blindroll`/`/selfroll` gated
  on the item above. (S)
- TODO: Composer sent-message history recall (S); chat bubbles over the speaking token (M);
  roll flavor text separate from the formula (S).
- TODO: Draw-without-replacement / consumable tables — the first write side effect of
  `draw_table`, which is read-only by design. (L, contested: a named non-goal)
- TODO: Table folders/grouping. (M-L, contested: FTS search may suffice)
- TODO: Typed note pages (image/PDF/video) as first-class `NoteEngine` kinds (M); heading TOC +
  in-note anchors (M); server-redacted per-recipient secret blocks — a new `Segment` variant in
  the shared chunk parser used by notes AND chat, stripped not hidden (L); per-user note
  sharing UI (M, scoping design fork: note-local vs shared component); "present to players"
  broadcast (S); live-resolved `doc:` link labels (S, contested: explicit labels are a
  documented decision).

### Audio, VFX, assets
- TODO: `PlaylistMode::Simultaneous` (one `PlayingTrack` per track under `MAX_PLAYING_TRACKS`)
  and `PlaylistMode::Soundboard` (`TrackEnded` removes the entry) in `audio::state::apply`, with
  play-all / per-track controls in `AudioPanel`/`PlaylistSheet`. (M)
- TODO: Stationary scene-placed sound source — `SoundEngine { x, y, elevation, emission:
  SoundEmission }` mirroring `LightEngine` exactly, fed into `compute_audibility` beside
  `token_sound_emission` through the same `segment_occluded`/`falloff`/`pan_for` primitives;
  new placeable tool. (L)
- TODO: Muffled occluded audio — an `AudibleEmitter.muffled` flag set where
  `AudioOverlay.through_wall_gain` is chosen; client inserts a lowpass `BiquadFilterNode`. (S-M,
  mildly contested)
- TODO: Perceptual volume curve at `AudioPanel`'s slider boundary only. (S, taste)
- TODO: Weather/particle effects layer — no particle system exists anywhere. (L+, own design
  pass)
- TODO: Camera pan-to-location/token animation. (scene tools; unsized)
- TODO: `TileEngine` — standalone raster/video scene props with `ElevationBand` occlusion plus a
  per-tile reveal parameter, and a `TileView` render layer; video textures must be cloned per
  instance. (L, occlusion shape contested)
- TODO: Scene texture preload with progress before a scene is "ready" (M, contested: lazy may
  be the better UX); random-variant token art over `AssetFilter` (M); asset-browser favorites
  (S) / list mode (M, contested); custom world fonts (L, contested: no consumer exists).

### Server, accounts, modules
- TODO: Self-service change-password UI (the admin-gated reset route ships now). (M)
- TODO: Live presence — an aux broadcast on a user's connection-count 0→1/1→0 transition;
  `Presence.svelte` renders static `ctx.members` today. (M)
- TODO: GM "reload connected clients" frame after `set_world_enabled_modules` — a loaded tab
  cannot execute a newly enabled module's JS (single-instance import map). (S-M)
- TODO: Account avatar/colour/pronouns columns in the baseline migration + wire types +
  settings UI. (M, confirm no collision with per-actor/token colour)
- TODO: `Config.route_prefix` for subpath reverse-proxy hosting — server routes, WS URL, asset
  URLs, client base path. (L)
- TODO: Intermediate world-role tiers vs extending `CapabilityGrants` to cover `require_gm`
  routes. (L, contested)
- TODO: Module load/activation failures reach the GM — `ModuleLoadResult.failed` and
  `ModuleRegistry.activate`'s skips are `logger.warn` only; surface via `AppContext.notify` and
  a per-module status line in `ModuleManager.svelte`. (S-M)
- TODO: `InstalledModuleInfo` carries the server's `modules::engine_compat_ok` verdict so
  `ModuleManager.svelte` can grey an incompatible module before Save (never a TS re-derivation).
  (S)
- TODO: Manifest `conflicts` field (M, mildly contested); pre-save dependency/contract
  satisfiability check in `ModuleManager.svelte` (S-M); module-provided starter content packs
  (L, contested: manifest field vs world-bundle import); generic module-scoped realtime channel
  — must carry document-style visibility tiering and rate limits, not a raw relay (L,
  contested against the document-only model); module-scoped persistent settings API (design
  question).
- TODO: Macro/quick-access hotbar — needs a scripting/sandboxing decision first. (design
  question)
- TODO: Compendium/pack support — `Scope::Compendium` round-trips but every route 404s it;
  `docs/design/M2-data-foundation.md` calls it deferred; `PLAN.md` Phase 4 now points here for the ruling. (L)
- TODO: Product-level folder nesting cap at `check_asset_folder_parent`/`check_note_parent`,
  separate from the 1,000-entry `MAX_MOVE_ANCESTRY` safety valve. (S, number is a UX call)

### UI shell
- TODO: Sibling drag-reorder for notes and asset folders — the only consumer today of
  `allocateSortKey`'s insert-between and `renumber` signal would be such a UI; every existing
  site only appends. (M, needs the drag-drop controller item below or a keyboard reorder)
- TODO: Progress-type in-place-updatable notification for chunked upload, conversion, backup.
  (M)
- TODO: One `ui-kit` confirm/prompt dialog primitive with autofocus, Enter-submit, one Escape
  path, distinguishable dismissal and a real Tab focus trap; replaces `Settings.svelte`'s raw
  `window.confirm()` and the per-site modals. (M)
- TODO: Click-anywhere-on-toast to dismiss (larger touch target). (S)
- TODO: Touch-first generic context menu (right-click AND long-press) extracted from
  `MenuKeyboard`; no context menu exists anywhere today. (M-L)
- TODO: Module keybindings framework in `@shadowcat/core` exposed via `ModuleContext`; migrate
  `module-ducking`'s one-off capture onto it. (M)
- TODO: `registerSetting` single-field convenience over `SETTINGS_SECTION_CONTRACT`. (S-M,
  contested: wait for demand)
- TODO: First-run onboarding tours. (L, product decision)

## Blocked on owner approval — invisible but contested improvements from the same survey

These change nothing a GM or player sees, but a reasonable engineer could argue against them,
so they wait for a ruling rather than shipping with the uncontested batch.

- TODO: Generic per-user `ClientMsg::Intent` flood budget in `ws::WsState` beside the existing
  per-surface limiters (partially self-throttled by the single-writer pool + `publish_guard`).
  (M)
- TODO: `deprecate(message, {since, until, once})` helper in `@shadowcat/core` for the
  module-facing API — adjacent to the Phase-4 module-API freeze. (S)
- TODO: Aggregate every engine-band `validate()` failure into one response instead of
  first-error short-circuit; client-side Zod already prevents most multi-error submissions.
  (M)
- TODO: A namespaced per-module metadata band beside `system` (size-capped, key-namespaced,
  opaque per namespace) so two modules attaching data to one document cannot collide — touches
  `REDACTABLE_BANDS`/`WRITABLE_BANDS`, the Zod mirror, and `writes_a_content_band`. (M-L,
  design question)
- TODO: One capture-phase Escape coordinator at the shell root — Escape is handled independently
  in nine places (`MenuKeyboard`, `MergeConflictModal`, `RollTooltip` ×2, `AssetPickOverlay`,
  `FolderTree`, `PreviewPane`, the dockview floating-panel listener, `ToolRail`) with no
  precedence; overlap behaviour would change and gestures cannot be regression-tested under
  jsdom. (M)
- TODO: One shared drag-and-drop controller replacing four hand-rolled implementations (three in
  `module-asset-browser`, `PanelHost` tab drag, `combat-tracker` pointer reorder) — same jsdom
  limit. (S-M)
- TODO: Scene-switch asset preload broadcast warming `AssetResolver` ahead of `Activate` —
  measure the stutter before building. (M)

## Owner-authorized deferrals: physical devices and manual browser UI (built-in HTTPS)
- TODO: Run the built-in HTTPS checks that automation cannot reach, on real hardware, and record
  the outcome in the hosting guide's browser table: (a) real Safari (iPhone, and macOS): `wss://`
  after accepting the self-signed certificate exception, the WebSocket-failure hint text, whether a
  typed `http://<lan-address>` is upgraded, and the loopback `ws://` refusal the Playwright WebKit
  build shows; (b) real Android Chrome: `wss://` after accepting the certificate exception and the
  WebSocket-failure hint text; (d) any browser's behaviour for a PUBLIC IP address typed as
  `http://`. The owner authorized deferring (a), (b) and (d): they need physical devices and a
  person driving browser UI, which nothing in the repo can simulate.

## Chromium HTTPS-First "continue" on macOS and Linux (not owner-authorized; awaiting CI)
- TODO: Confirm the macOS and Linux drivers of the Chromium HTTPS-First prompt on the CI runners.
  The prompt is browser UI, not web content, so only the operating system's accessibility API
  reaches it (DevTools DOM, the accessibility tree, mouse and keyboard input and clicks at the
  button's measured position all failed to reach it). `e2e-support/browser-ui.ts` dispatches to
  Windows UI Automation (run against a real browser), macOS System Events through JavaScript for
  Automation (`invoke-button.jxa`) and Linux AT-SPI through PyGObject (`invoke-button.py`); the
  `ui-e2e-https` job runs the interactive test on all three systems (Linux under a virtual display,
  a session bus and the accessibility bus). Neither non-Windows driver could be run from the
  Windows development machine. The first CI run must show, per platform: the test
  "HTTPS-First's 'Continue to site' reaches the plain-HTTP app" passes (the button is found by the
  pinned English name, invoked, and the plain-HTTP login form appears). If a driver fails, the
  failure text names the step (no accessibility access on macOS, no AT-SPI application for the
  browser's process id on Linux, button name not found) and the driver is fixed from it; only a
  platform that cannot be driven after that may be proposed to the owner as a conditional deferral.
