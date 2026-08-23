# TODO — Deferred Work

Actionable, externally-logged deferrals. Bugs go in `OPEN_BUGS.md`, not here.
As of the Phase-1 cleanup burndown (2026-07-19), most items below are
retained because their blocking capability doesn't exist yet — a concrete
unblocking condition, not a "someday maybe." A few headings are explicitly
labeled "Actionable now": these are NOT blocked on anything — the underlying
capability already exists — but are deferred as out-of-scope-for-now work.

## Blocked on a per-turn movement-budget system (Phase-2 combat)
- TODO: `move_exec::execute_move`'s `MoveOutcome.cost` accumulates only the entered cell's terrain multiplier per step (`cost += regions.terrain_multiplier(region_cell)`); the `pathfinding` module's router cost also multiplies by the diagonal-rule `step_cost` (`sc * mult`, where `sc` is 1.0/2.0/√2/alternating depending on `world-settings.pathfinding.diagonalRule`). The two "cost" values are not numerically comparable once diagonal movement is involved under any non-Chebyshev rule — they coincide only because Chebyshev's diagonal step cost is 1.0. This is a deliberate M10g Task 7 scoping decision (`move_exec`'s center-cell, terrain-only accounting model), not an oversight, and nothing currently consumes or compares the two values. Resolve before any per-turn movement-budget system consumes `MoveOutcome.cost`/`MoveStream.cost`: decide whether `move_exec` should thread the diagonal rule + per-step parity to match the router's preview cost, or whether route-preview cost and execution cost are intentionally distinct quantities. (Surfaced by the M10g Task 7 buddy check.)
- TODO: `navmesh::los_smooth` (M10f-4) reports the smoothed continuous route's `cost` as the PRE-smoothing weighted grid cost, unchanged — it does not recompute an exact per-span cost for the straightened any-angle chords, only guarantees the reported value is a conservative (never cheaper) budget preview. Same preview-vs-execution divergence class as the `MoveOutcome.cost`/router-cost split logged above: a per-cell-exact smoothed continuous cost is deferred, not implemented. Resolve alongside the item above if a per-turn movement-budget system ever needs an exact continuous-engine cost.

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

## Follow-on feature sub-projects (own brainstorm → spec → plan each)

Out of scope for the Phase-1 cleanup burndown; built after Sub-project 1, one design pass each
(user: build ALL of bucket C):

1. **Real-time per-recipient move-streaming** — `MoveStream` precomputes each move's
   per-recipient vision clip at execute time, so two tokens moving simultaneously do not reveal
   each other mid-walk when a watcher's vision opens after the clip; it reconciles only at the
   stop + next `vision` rebroadcast. No correctness/secrecy impact today — only a missed
   transient reveal. Needs a per-move server loop recomputing each recipient's visibility of
   every concurrently moving token as positions advance, replacing execute-time precompute.
   **BLOCKED ON USER INPUT** — see `docs/superpowers/specs/2026-08-21-realtime-move-streaming-
   design.md`; the buddy-checked design shows the natural fix needs new infrastructure comparable
   in cost to the alternative it was meant to avoid.

## Registered and exercised — plugin distribution properties
Registration is per-machine state no committed file can carry. The supported non-interactive path
is the `claude plugin` CLI, not only the TUI command:
`claude plugin marketplace add <your Shadowcat checkout>`, then
`claude plugin install shadowcat-codebase@shadowcat --scope project` run from the consumer repo.
Scope matters — `user` scope would enable it in Shadowcat too and double-register every skill and
agent name.

Per-machine setup actions inside a CONSUMING repo's workspace belong to that repo, not here: a
consumer workspace's trust dialog is tracked in that repo's own backlog.

Settled by direct observation, kept because each is a property to re-check after a refresh:
- A directory-sourced plugin serves a cached SNAPSHOT, not the live repo — CONFIRMED. The payload
  lands at `~/.claude/plugins/cache/<marketplace>/<plugin>/<version>/` as real copied files, and
  the install record pins a `gitCommitSha`. Editing a skill in Shadowcat therefore does NOT reach
  a consumer until `version` in `.claude/.claude-plugin/plugin.json` is bumped and the marketplace
  updated; the "stored exactly once, drift-free" property holds on disk but not at runtime.
- The copy is the whole directory, ignore rules notwithstanding: `settings.json`,
  `settings.local.json`, `kimi.plugin.json` and `graphify/` all ship. The loader consumes only
  `skills/`, `agents/`, `hooks/`, `commands/`, so the rest is inert — but a machine-local
  `settings.json` sitting in a distributed payload is worth re-checking if the loader ever widens.
- Plugin skills ARE addressed under a `<plugin>:<skill>` prefix — the listing offers only
  `shadowcat-codebase:shadowcat-codebase-core`, never the bare id. The agent bodies already name
  this case, and invocation was verified end-to-end from a consumer session.
- The routing hook fires exactly once per session: a consumer that declares no `hooks` key of its
  own gets only the plugin's, while Shadowcat keeps its own wiring and shows the plugin disabled.
- `claude plugin validate` warns that a `CLAUDE.md` at the plugin root is NOT loaded as project
  context. A consumer therefore needs its own adapted `.claude/CLAUDE.md` rather than inheriting
  one — do not "simplify" by deleting it in favour of the shipped copy.
- Both manifests validate with one cosmetic warning each: no `author` field.

## Actionable now — Kimi Code parity is written but never installed
- TODO: In the Kimi TUI run `/plugins install <your Shadowcat checkout>/.claude` then `/reload` (the
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
