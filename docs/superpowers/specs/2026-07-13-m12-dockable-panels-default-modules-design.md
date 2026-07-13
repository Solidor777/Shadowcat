# M12 — Dockable Panel System + Minimal Default Modules (cross-cutting design)

**Status:** approved (user, 2026-07-13; sections 1–3 approved interactively, decomposition approved).
**Supersedes:** the one-line M12 entry in `docs/PLAN.md` ("Minimal default modules").
**Execution directive (user, 2026-07-13):** design/specs on the design-session model mainline;
execution via subagent-driven development with Sonnet implementers (`shadowcat-coder`) and the
standard two-reviewer gates; pause only for design/usage questions this spec does not answer.
Push to origin remains a user decision (M11 body is still unpushed).

## 1. Goal

Every major UI element is already a module (M8.5 discipline); M12 makes the *arrangement* of those
elements user-owned, and ships the first document-centric default modules against the public API:

1. A **unified dockable panel system**: one `panel` primitive with five presentation states —
   docked / floating / minimized / popped-out / compact-view — replacing the fixed sidebar.
2. **Sheet registry + generic sheets** (actor, item, fallback) and `ctx.openDocument`.
3. **Actor + scene browsers**, unlocking **multi-scene** (the PLAN-parked deferral).
4. A **layout refresh**: topbar launcher, statusbar dock strip, real mobile tooling.
5. **Pop-out windows** (pulled forward from Phase 2 by user decision, as the final checkpoint).

Each sheet/browser is built ONLY against public seams; every friction point is logged as an API
bug report (PLAN's standing M12 rule). The M7 token-set re-audit rides along with the first
sheets/browsers (PLAN M7 note).

## 2. Decisions locked (user, 2026-07-13 — do not re-litigate)

- **D1 — Model A**: floating windows + dock discipline over docked-tiling-only or sidebar-stack.
- **D2 — Unified panel primitive**: sidebar, windows, sheets are ONE primitive with presentation
  states; today's tabbed sidebar is "a tabbed group docked right," not special chrome.
- **D3 — Default layout is chat-only** docked right; all other panels launch closed (topbar).
- **D4 — Full docking scope in v1**: edge zones (right/bottom/left) · tabbed groups · linear
  splits within zones · drag-to-dock with previews · floating · minimize-to-chip · **pop-out**.
  Pop-out ships as M12's own final sub-checkpoint (M12e) so it cannot destabilize the rest.
- **D5 — Engine**: adopt **dockview-core** wrapped behind a project-owned contract, **gated on a
  source-verification spike** ([[verify-crate-claims-against-vendored-source]]). Spike FAIL on a
  gating question ⇒ bespoke engine behind the same contract; only the wrapper module changes.
- **D6 — Multi-scene** ships with the scene browser (M12d) via an `activeScene` world-settings
  field; players follow, GM roams. No new server code.

## 3. The panel model

A **panel** is plain registration data + a Svelte component:

```ts
interface PanelRegistration {
  id: string;                    // "chat", "factions", "sheet:<docId>" (runtime instances)
  icon: string;                  // emoji/icon token, host-rendered
  labelKey: string;              // i18n key, HOST-resolved (locale-reactive) — M11d-1 precedent
  gmOnly?: boolean;              // advisory UI filter ONLY; server redaction is the real gate
  defaultPlacement?: Placement;  // zone/group/order or floating rect; absent ⇒ launcher-only
  component: PanelComponent;     // mounted once, presentation-agnostic
}
```

**Presentation states** (per panel instance): `docked(zone, group, tabIndex)` ·
`floating(rect, z)` · `minimized` (dock chip) · `popped-out(windowRef)` · and on compact
viewports, `view` (full-screen entry in the switcher). States are a property of the *layout*,
never of the panel component — components cannot know or branch on where they are hosted.

**Two panel classes:**
- **Registry panels** — singleton, module-registered at activation (chat, assets, actors,
  factions, conditions, game-settings, settings; combat tracker joins in Phase 2).
- **Document panels** — multi-instance, created at runtime by the sheet registry
  (`sheet:<docId>`); floating by default; re-open ⇒ focus existing, never duplicate.

**Hard invariants:**
- **Keep-mounted discipline**: hidden/inactive panels hide via CSS (`hidden`), never `{#if}`
  (M11d-1 mount-counter precedent, extended). Dock⇄float⇄pop-out transitions **re-parent** the
  panel's DOM element; they never destroy/recreate. Chat scroll position and composer drafts
  survive being dragged anywhere. (Engine re-parenting behavior = spike gate question.)
- **The stage is a locked center well**: mounted from the existing `shadowcat.surface:stage`
  provider into the panel host's center; never closable, never a drop target, never covered by a
  *docked* zone. Floating panels may overlay it.
- **Panels talk only through seams** (ARCHITECTURE §2 invariant 7) — a panel never imports the
  panel manager, the shell, or another panel.

## 4. Architecture — modules, contracts, seams

### 4.1 `@shadowcat/module-panels` (new; replaces `module-sidebar`)

Exactly the swap the M11d-1 `sidebar-host` seam was designed for. Provides:
- the singleton **`shadowcat.surface:panel-host`** surface (hosted by core-ui's `main` region);
- the dock-chip strip contributed into a new **`shadowcat.surface:panel-dock`** surface hosted by
  the statusbar;
- collection of **`shadowcat.panel:*`** registrations (a new multi contract family mirroring the
  contribution model's shape);
- the engine wrapper (dockview-core or bespoke): dock tree (zones → tabbed groups → linear
  splits), floating layer + z-order/focus, minimize, pop-out host, drag-to-dock with drop
  previews;
- the **compact switcher** (see §8);
- **persistence** (see §7).

The engine is invisible: **zero engine types appear in any contract or AppContext seam.**

### 4.2 Retired / changed contracts

- `shadowcat.surface:sidebar` (multi) and `shadowcat.surface:sidebar-host` (singleton) **retire**.
  `module-sidebar` is deleted; `TabbedSurface` remains in ui-kit only if something else still uses
  it (expected: nothing — verify, then delete; churn is authorized).
- The 7 existing panel modules re-target their contributions from sidebar tabs to
  `shadowcat.panel:*` registrations. Component internals untouched. Former tab orders become
  launcher ordering: chat 0, assets 1, actors 2, factions 3, conditions 4, game-settings 5
  (gmOnly), settings 6.
- `Contribution.tab` metadata is superseded by `PanelRegistration`; remove `tab` after the
  re-target (no dangling dual mechanism).

### 4.3 AppContext seams (narrow, engine-free)

```ts
ctx.panels: {
  open(id: string): void;      // restore saved/default placement; already open ⇒ focus
  close(id: string): void;
  focus(id: string): void;
  toggle(id: string): void;
}
ctx.openDocument(ref: { docId: string } | { tokenId: string }): void;  // §5
```

`ctx.uiState` grows `getPanelLayout(world)/setPanelLayout(world, blob)` alongside the existing
activeTab accessors (which retire with the sidebar).

### 4.4 Shell/region changes (M12b)

- **core-ui Layout**: grid becomes `topbar / toolrail / main / statusbar`; `main` hosts
  `panel-host`. The `min-height: 0; overflow: hidden` growth-cap discipline moves to the panel
  host's panes (same invariant, new owner).
- **module-topbar**: gains the **launcher** — registered panels (gmOnly-filtered) + overflow menu
  + world/scene title + presence (roster is role-wide since M11d-1) + settings entry.
- **module-statusbar**: hosts `panel-dock`; height 1.5rem → 2rem for chip legibility/touch.
- **module-toolrail** (scene-tools' rail): desktop unchanged; compact renders a bottom tool strip
  (presentation-only — tools already sit behind `SceneToolHost`). Replaces today's
  `display: none` on phones.

## 5. Sheet registry, `openDocument`, generic sheets (M12c)

### 5.1 Registry

New multi contract family **`shadowcat.sheet:<doc_type>`**; providers register
`{ component, priority: number, match?(doc): boolean }`. Resolution: filter by doc_type → apply
`match` → highest `priority` → deterministic tie-break (lexicographically lowest provider module
id — the M11d-3 deterministic-singleton precedent). An always-registered **generic fallback
sheet** (priority −∞, any doc_type) guarantees every document can open.

### 5.2 `ctx.openDocument(ref)`

1. Resolve target: `docId` → that document. `tokenId` → **linked token ⇒ its actor document;
   instanced token ⇒ the token's embedded actor**, with writes addressed to the resolved write
   site (`/system/...` on the actor vs `/embedded/actor/0/system/...` on the token) — the
   `resolveTokenActor`/`conditionTarget` precedent. Fail-closed: dangling refs open nothing and
   log; never a crash.
2. Resolve sheet via §5.1.
3. Open/focus document panel `sheet:<docId>` (floating default placement, cascade offsets).

### 5.3 Generic sheets

- **Actor sheet** (`@shadowcat/module-sheet-actor`): engine-known fields as real controls —
  portrait/visual, name (privacy-aware via `actorDisplayName`; players see the redacted name),
  faction, size/shape, vision modes, speed, conditions — plus the opaque `system` body as a
  **type-aware tree editor** (string/number/bool/object/array; add/remove fields), plus the
  embedded-items inventory list (open item ⇒ `openDocument`).
- **Item sheet** (`@shadowcat/module-sheet-item`): **`item` becomes a first-class doc_type**
  (client semantics only; server stays structural — no server change). Items live standalone or
  embedded in an actor (inventory); write-site resolution as in §5.2. Dice-notation string values
  get a roll-to-chat affordance posting `/roll` over the M11 wire.
- **Fallback document sheet**: envelope metadata + the same `system` tree editor.

**Sheet data rules (hard):** sheets read the **optimistic store** (per-recipient redaction and
OwnerOrGm naming come free; [[render-from-optimistic-view]] holds for panels too). Edits are
field-path Updates through the normal optimistic path **with real OCC pre-images** — never
`old: null` (the M11d-2 GameSettingsPanel Critical, promoted to a spec requirement). Editability
is gated by advisory `AppContext.canEdit` (read-only rendering for unauthorized users); the
server remains authoritative.

### 5.4 Chat deferral closure

Actor names on chat cards and internal doc-link segments become `openDocument` links
(permission-gated: no READ ⇒ plain text, no link) — closing the M11d-1 TODO items that were
blocked on M12 sheets.

## 6. Browsers + multi-scene (M12d)

- **Actor browser**: `module-actors`' panel grown — live FTS search (M6c subscription seam),
  create, open-sheet, place-on-stage via the existing `ActorSelection` seam.
- **Scene browser** (`@shadowcat/module-scene-browser`, GM-gated panel): scene list with
  thumbnails (background asset), create, configure (opens the scene's sheet — the game-settings
  per-scene sections become reachable per scene), **activate**.
- **Multi-scene**: `activeScene: string | null` on the `world-settings` config doc (GM-writable
  via the normal config-doc path). Players' `WorldSession` subscribes to `activeScene`
  (fail-closed to current behavior — first scene — when absent). The GM may locally view any
  scene (their subscription is their own; see-as and vision channels are already per-scene).
  Scene *deletion* stays deferred (pre-M10 deferral stands).

## 7. Persistence

`ui_state.worlds[world].panelLayout` (client-owned structure; server still validates only
object+size cap):

```jsonc
{
  "version": 1,
  "expanded": { "zones": { "right": [ { "tabs": ["chat"], "active": "chat", "size": 0.5 } ], ... },
                 "floating": [ { "id": "...", "rect": {...}, "z": 3 } ],
                 "poppedOut": ["chat"], "minimized": ["sheet:..."] },
  "compact":  { "activeView": "chat", "order": ["chat", "..."] }
}
```

Written via the existing leading-edge-debounced PUT. **Fail-safe:** unparseable/unknown-version
blob ⇒ reset to default layout + toast (never a wedged shell). Unknown panel ids (module removed)
prune silently with a debug log. Document panels persist as minimized/floating entries only if
their document still resolves at restore; otherwise pruned.

## 8. Compact (mobile) presentation

One breakpoint axis: **compact < 48rem ≤ expanded** (single source of truth in ui-kit; replaces
the ad-hoc 40rem media query). On compact the dock tree is **ignored, not interpreted**: the
panel registry renders as full-screen views behind a bottom switcher; chat is additionally
reachable as a swipe-up drawer on the map view; document panels join the switcher as views.
Only `compact.activeView`/`order` persist for compact. Touch drag-to-dock is not required
anywhere: the §9 command menu is the touch path on expanded-width tablets.

## 9. Accessibility (requirements, not aspirations)

- **Every drag has a command equivalent**: each panel header/tab exposes a menu — *Move to right
  dock / bottom dock / left dock · Float · Minimize · Pop out · Close* — which is simultaneously
  the keyboard path, the screen-reader path, and the touch fallback.
- Floating panels: non-modal `role="dialog"` with `aria-label`; focus moves in on open, restores
  to trigger on close; Escape closes the focused floating panel.
- Tab strips: APG tabs pattern with **roving tabindex** (picks up the M11d-1 deferral — we are
  rebuilding that host).
- Dock chips: real buttons with accessible names; dock/undock/minimize actions announce via a
  polite live region.
- Focus ring via `:focus-visible` on every interactive part; targets ≥24 CSS px (~44 on coarse
  pointers).

## 10. Error handling

- Invalid persisted layout ⇒ fail-safe default + toast (§7).
- Pop-out blocked (popup blocker; pop-out MUST be triggered from a user gesture) ⇒ panel falls
  back to floating + notice.
- **Every panel body mounts inside `<svelte:boundary>`** — a crashing panel component (this is
  the moddable surface) renders "panel crashed · reload" in place; the shell survives.
- Engine exceptions during drag/drop are caught at the wrapper boundary; layout state is only
  committed from validated tree transitions.

## 11. Engine adoption gate (M12a-0 spike)

Adoption of dockview-core proceeds ONLY if the source spike (pinned version, real source, file:line
citations) PASSes the gating questions; **an unverifiable claim counts as FAIL**:

- **Gating:** framework-agnostic DOM content API; DOM **re-parenting** (not recreate) on
  dock⇄float⇄group moves; pop-out via same-heap `window.open` with element re-parenting; a
  lockable non-closable, non-drop-target center group (stage well); full-layout serialization
  round-trip incl. floating; MIT license.
- **Informative (shape the wrapper, don't gate):** pointer-event/touch support, built-in keyboard/
  ARIA, CSS-var theming, bundle weight, dependency count, maintenance cadence.

FAIL ⇒ bespoke engine inside `module-panels` implementing exactly §3–§4 (same contract, no other
module changes). The spike report is committed under `docs/superpowers/specs/`.

**Outcome (2026-07-13): ADOPT `dockview-core@7.0.2`** — see
[`2026-07-13-m12a-dockview-core-spike.md`](2026-07-13-m12a-dockview-core-spike.md). 13 PASS /
1 PARTIAL: the non-drop half of the stage-well question is native + verified; the non-closable
half has no engine primitive and converts into mandatory M12a wrapper requirements **W1–W3**
(headerless stage group; `onWillDrop` veto + wrapper API refusal for the stage id; fail-safe
restore-if-removed guard, each with dedicated tests). Gating citations were independently
re-verified mainline against the cloned source.

## 12. Testing requirements

- **Layout tree = pure reducer**: unit tests for dock/undock/split/minimize/restore/focus ops,
  serialization round-trip, unknown-id pruning, version fallback.
- **Registry resolution**: priority, `match`, deterministic tie-break, fallback-always-resolves.
- **Mount discipline**: mount-counter test across dock⇄float⇄tab-switch⇄minimize⇄restore
  (extends the M11d-1 guard); a `svelte:boundary` crash-containment test.
- **Compact switcher**: view rendering, drawer, persistence.
- **Sheets**: write-site resolution (linked vs instanced), OCC pre-image correctness (a test that
  fails on `old: null`), read-only rendering for non-editors, tree-editor round-trip.
- **Multi-scene**: player follows `activeScene` flips; GM local view unaffected; absent field ⇒
  legacy first-scene behavior.
- Per-task `pnpm -r typecheck` ([[vitest-skips-typecheck-in-sdd]]) and the full lint/test gates.

## 13. Decomposition (each = its own plan → SDD execute cycle; sequential)

| Checkpoint | Scope | Notes |
|---|---|---|
| **M12a** | Spike gate (M12a-0), then `module-panels` core: zones/groups/tabs/splits, floating, minimize, drag-to-dock, persistence, compact switcher; sidebar swap; 7 modules re-targeted; chat-only default; command-menu a11y path | The big one; buddy-check the layout reducer + the swap |
| **M12b** | Layout refresh: topbar launcher, statusbar dock strip, mobile tool strip, token/density re-audit | |
| **M12c** | Sheet registry + generic actor/item/fallback sheets + `openDocument` + chat doc-link closure | item doc_type introduced here |
| **M12d** | Actor + scene browsers; multi-scene `activeScene` | closes the PLAN multi-scene deferral |
| **M12e** | Pop-out windows | pulled forward from Phase 2 (user decision) |

M12.5 (backups + snapshot restore) follows unchanged, then the dogfood-alpha gate.

## 14. Exclusions

- Region drag-resize of topbar/toolrail/statusbar chrome, multi/user themes, module styling modes
  (still Phase 2 "layout/theming completion" — M12 removes only `pop-out` from that line).
- Combat tracker (Phase 2; it will register as a panel with zero panel-system changes).
- List virtualization beyond existing caps; unread badges (TODO.md items stand).
- Scene/world deletion (pre-M10 deferral stands).
- Server-side layout validation beyond the existing ui_state object+size cap.

## 15. Process notes

- Reviewed skill-update gate: M12a/b touch `shadowcat-codebase-client-shell` (and likely create a
  new `shadowcat-codebase-panels` skill — decide at the gate; a new subsystem without a skill is
  itself a gate violation). M12c/d touch documents-permissions (item doc_type semantics) and
  scene-rendering (activeScene) skills.
- API friction found while building sheets/browsers is logged in `docs/POST_WORK_FINDINGS.md` as
  API bug reports (PLAN's M12 rule), then triaged.
- Buddy-check pre-authorizations (fold into plans): the M12a layout reducer + sidebar swap; the
  M12c write-site resolution + OCC pre-image path; the M12e pop-out same-heap lifecycle.
