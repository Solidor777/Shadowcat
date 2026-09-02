# TODO — Deferred Work

Actionable, externally-logged deferrals. Bugs go in `OPEN_BUGS.md`, not here.
As of the Phase-1 cleanup burndown (2026-07-19), most items below are
retained because their blocking capability doesn't exist yet — a concrete
unblocking condition, not a "someday maybe." A few headings are explicitly
labeled "Actionable now": these are NOT blocked on anything — the underlying
capability already exists — but are deferred as out-of-scope-for-now work.

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

1. **Real-time per-recipient move-streaming** — DONE (2026-08-27, spec
   `docs/superpowers/specs/2026-08-27-move-stream-live-clip-design.md`): observer's own-move
   timeline clip + re-emit; the moving-light residual is delivered by M17d (`HISTORY.md`).

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

## Actionable now — next file-size split candidate
- TODO: `src/server/src/data/sqlite.rs` production code is ~3,900 lines after its test module moved out — the largest remaining production file and the next to cross the 5,000-line soft limit at its growth rate. Split `SqliteRepository` by concern (documents/commands, membership/invites, search, world export/import) into `data/sqlite/<concern>.rs` `impl` blocks before it reaches the limit; the gate (`pnpm lint:file-size`) fails the build at that point and no allowlist entry is to be added.
