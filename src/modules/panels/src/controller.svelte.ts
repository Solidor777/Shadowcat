// The layout-owning controller for the panel-manager host. Bridges every
// layout-changing input — engine gestures (drag/dock/float/minimize) AND the
// imperative `PanelsApi` any module can invoke via `ctx.panels` — onto the pure
// `applyOp` reducer, then persists the result. `PanelHost` owns the DOM/engine
// adapter and the compact/expanded presentation switch; this class owns
// nothing but layout state, registration visibility, and the persistence
// round-trip, so it is usable and testable without mounting any component.
import { PANEL_CONTRACT, type Contribution, type ContributionRegistry, type Logger, type PanelMeta } from "@shadowcat/core";
import type { WorldRole } from "@shadowcat/types";
import type { PanelsApi, PanelsChipsView } from "@shadowcat/ui-kit";
import { createSubscriber } from "svelte/reactivity";
import { applyOp, defaultLayout, locate, placeNewRegistrations, prune, type LayoutOp, type PanelLayoutV1, type PopoutWindowLayout } from "./layout/tree";
import { decodeLayout, encodeLayout } from "./layout/persist";

/** gmOnly filtering is ADVISORY ONLY: it hides a panel from this client's own
 * UI, exactly like `PanelHost`'s prior inline filter. It is not a security
 * boundary — the server remains the sole authority over any data a gmOnly
 * panel displays or writes.
 * @param regs Candidate `shadowcat.panel` contract registrations to filter.
 * @param role The current world session's role.
 * @returns `regs` with every `gmOnly` registration removed for a non-`"gm"` role.
 * @example
 * ```ts
 * import { regsForRole } from "@shadowcat/module-panels";
 * import type { Contribution } from "@shadowcat/core";
 *
 * declare const regs: readonly Contribution[];
 * const visible = regsForRole(regs, "player");
 * ```
 */
export function regsForRole(regs: readonly Contribution[], role: WorldRole): Contribution[] {
  return regs.filter((c) => !(c.panel?.gmOnly && role !== "gm"));
}

/** The narrow shape this controller needs from the shell's `PanelsBridge` —
 * `bind` is not part of `PanelsApi` itself (a proxy-rebind affordance, not
 * something a terminal implementation like this controller needs to expose),
 * so callers hand it in separately from the `PanelsApi` methods below. */
export interface PanelsBridgeLike {
  /** Installs `impl` as the bridge's live `PanelsApi` implementation.
   * @param impl The implementation to bind.
   */
  bind(impl: PanelsApi): void;
}

/** Everything `PanelsController` needs from its host to bridge layout-changing inputs onto
 * `applyOp` and persist the result. */
export interface PanelsControllerDeps {
  /** The registry to read `shadowcat.panel` contract registrations from; keys panels off
   * `Contribution.id` (see `Contribution`'s own doc for the uniqueness caveat this
   * controller does not itself verify). */
  contributions: ContributionRegistry;
  /** The current world session's role, consulted by `regsForRole` to filter `gmOnly`
   * registrations. */
  role: WorldRole;
  /** Reads the persisted layout blob (`unknown` — the shell's `panelLayout` storage is
   * Zod-free by design). */
  getPanelLayout: () => unknown;
  /** Persists a new layout blob. */
  setPanelLayout: (blob: unknown) => void;
  /** The shell's `PanelsBridge`, into which this controller binds itself as the live
   * `PanelsApi` implementation. */
  bridge: PanelsBridgeLike;
  /** Diagnostic sink for recoverable failures. */
  logger: Logger;
  /** Fired with the `panels.layoutReset` i18n key when a persisted layout blob
   * was rejected and the default was substituted in its place; the caller
   * resolves + surfaces it (e.g. a statusbar live-region toast). */
  onReset?: (key: string) => void;
  /** Fired with an op and the layout as it stood immediately BEFORE that op,
   * every time `dispatch` actually changes the layout (never for a no-op
   * op) — drag gestures and `PanelMenu` commands both funnel through
   * `dispatch`, so this is the ONE hook that sees every layout-changing op
   * regardless of origin. `PanelHost` uses `prev` to distinguish a real
   * placement change from a focus bump (e.g. `"open"` on an already-docked
   * panel) when it drives the a11y live-region announcement. */
  onOp?: (op: LayoutOp, prev: PanelLayoutV1) => void;
  /** Fired with a user-facing i18n key for a CONTROLLER-originated notice the
   * caller surfaces (live region / toast) — today, only ever
   * `panels.popoutRestoredFloating`, queued by `#rehydratePoppedOut` when
   * reload rehydrates a popped-out panel to floating (a page load cannot
   * reopen a popup) and flushed post-mount via `flushPendingNotice`. An
   * ENGINE-originated notice (e.g. `DockviewEngine`'s `panels.popoutBlocked`)
   * is a SEPARATE channel: `PanelHost` subscribes to `EngineAdapter.onNotice`
   * directly and never routes it through this hook. */
  onNotice?: (key: string) => void;
}

const EMPTY_LAYOUT: PanelLayoutV1 = {
  version: 1,
  expanded: { zones: { right: { groups: [], size: 320 }, bottom: { groups: [], size: 240 }, left: { groups: [], size: 320 } }, floating: [], minimized: [], popouts: [] },
  compact: { activeView: null, order: [] },
};

// Cascade base/step for the floating rect `#rehydratePoppedOut` assigns a persisted
// popped-out id when it converts that id to floating on reload — an unoffset rect
// would stack every rehydrated panel (and the first-ever floating panel) at the
// identical (x,y). Value kept numerically aligned with
// `layout/tree`'s SHEET_CASCADE_BASE/STEP (not imported — that pair is layout-internal;
// this is the controller's own, deliberately separate constant so the two modules
// stay decoupled) so the SAME logical operation (reload -> float a persisted
// popout) lands at the same screen position regardless of which of the two call
// sites (already-registered vs late-registering panel) handles a given panel.
const REHYDRATE_FLOAT_BASE = { x: 96, y: 96, w: 420, h: 520 };
const REHYDRATE_FLOAT_STEP = 28;

/** The layout-owning controller: reduces every layout-changing input — engine
 * gestures and the `PanelsApi` methods below — onto the pure `applyOp`
 * reducer, then persists the result via `deps.getPanelLayout`/`setPanelLayout`.
 * Implements both `PanelsApi` (the general per-panel surface any module can
 * call through `ctx.panels`) and `PanelsChipsView` (the minimized-chip strip's
 * read/restore surface) — `PanelHost` binds one instance into the shell's
 * `PanelsBridge` at mount (see `PanelsBridgeLike`).
 * @example
 * ```ts
 * import { PanelsController } from "@shadowcat/module-panels";
 * import type { PanelsControllerDeps } from "@shadowcat/module-panels";
 *
 * declare const deps: PanelsControllerDeps;
 * const controller = new PanelsController(deps);
 * ```
 */
export class PanelsController implements PanelsApi, PanelsChipsView {
  /** The host-supplied dependencies this controller bridges layout-changing inputs through. */
  #deps: PanelsControllerDeps;
  /** `createSubscriber`'s notify function for non-rune (`PanelsChipsView`) consumers of
   * `#layout`. */
  #subscribe: () => void;
  /** The live layout state; the sole value every `PanelsApi`/`PanelsChipsView` method reads
   * and every layout-changing input mutates via `applyOp`/`prune`/`placeNewRegistrations`. */
  #layout = $state<PanelLayoutV1>(EMPTY_LAYOUT);
  /** The pre-`prune` structurally-validated blob `#layout` was decoded from at construction
   * (`null` on reset/first-run) — retained so `syncRegistrations` can place a
   * later-registering panel at its ACTUAL persisted location instead of `reg.placement`'s
   * static default. See `decodeLayout`'s `source` field and `placeNewRegistrations`'s
   * `persistedSource` parameter for the mechanism; this is what stops a real saved layout
   * from being silently overwritten with defaults whenever a panel module's own
   * registration happens to run after the `panels` module's (routine, since every panel
   * module `requires` `PANEL_CONTRACT`, which topologically activates `panels` FIRST). */
  #persistedSource: PanelLayoutV1 | null = null;
  /** A notice queued during construction, pending `flushPendingNotice()` —
   * see that method's doc comment for why `#rehydratePoppedOut` cannot call
   * `deps.onNotice` directly. */
  #pendingNotice: string | null = null;

  /** Decodes the persisted layout blob (falling back to a fresh
   * `defaultLayout` built from the role-filtered registry on garbage/absence —
   * see `decodeLayout`), rehydrates any persisted popped-out panel to floating
   * (`#rehydratePoppedOut`, since a page load cannot reopen a real popup), and
   * binds this instance into `deps.bridge`.
   * @param deps The controller's dependencies (contribution registry, role,
   * layout persistence hooks, bridge, logger, and the optional
   * reset/op/notice callbacks).
   * @example
   * ```ts
   * import { PanelsController } from "@shadowcat/module-panels";
   * import type { PanelsControllerDeps } from "@shadowcat/module-panels";
   *
   * declare const deps: PanelsControllerDeps;
   * const controller = new PanelsController(deps);
   * ```
   */
  constructor(deps: PanelsControllerDeps) {
    this.#deps = deps;
    this.#subscribe = createSubscriber((update) => deps.contributions.subscribe(update));

    const regs = regsForRole(deps.contributions.contributionsFor(PANEL_CONTRACT), deps.role);
    const known = new Set(regs.map((c) => c.id));
    const buildDefault = () => defaultLayout(regs.map((c) => ({ id: c.id, placement: c.panel?.defaultPlacement })));
    const { layout, reset, source } = decodeLayout(deps.getPanelLayout(), known, buildDefault);
    this.#layout = layout;
    this.#persistedSource = source;

    if (reset) {
      this.#persist(layout);
      deps.logger.warn("panels: persisted layout blob was invalid; reset to default");
      deps.onReset?.("panels.layoutReset");
    }

    this.#rehydratePoppedOut();

    deps.bridge.bind(this);
  }

  /** Popouts cannot be reopened without a user gesture (a page load is not one
   * — the browser blocks it), so every persisted pop-out window's panels
   * rehydrate to floating at construction, before the first `apply()`. The
   * tree's `popouts` records persist across sessions; the live `Window` never
   * does. Panels float window-by-window (each window's `panels` in tab order)
   * so one saved window's panels cascade ADJACENTLY, and each converted
   * window's entry is RETAINED — marked `dormant`, rect and panel set intact —
   * as the arrangement record a later restore gesture re-opens (see
   * `PopoutWindowLayout.dormant`).
   * Runs once; persists + queues a notice only if it actually converted
   * anything — see `flushPendingNotice` for why the notice is QUEUED here
   * rather than fired through `deps.onNotice` directly (this method runs
   * synchronously inside the constructor, before the host has mounted).
   * @example
   * ```
   * // private method; not part of the public API — invoked only from the
   * // constructor, once, before deps.bridge.bind(this) runs
   * this.#rehydratePoppedOut();
   * ```
   */
  #rehydratePoppedOut(): void {
    const live = this.#layout.expanded.popouts.filter((w) => w.dormant !== true);
    if (live.length === 0) return;
    let l = this.#layout;
    for (const w of live) {
      for (const id of w.panels) {
        // Cascade off the CURRENT floating count each iteration (not the loop
        // index) so rehydrated popouts interleave correctly with any panel
        // that was already floating before rehydration ran.
        const n = l.expanded.floating.length;
        const off = (n % 6) * REHYDRATE_FLOAT_STEP;
        const rect = { x: REHYDRATE_FLOAT_BASE.x + off, y: REHYDRATE_FLOAT_BASE.y + off, w: REHYDRATE_FLOAT_BASE.w, h: REHYDRATE_FLOAT_BASE.h };
        l = applyOp(l, { op: "float", id, rect });
      }
    }
    // Retain the arrangement record: each converted window's entry is written
    // back marked dormant. Its panels are now floating, so `locate` never
    // resolves them through the entry; the entry exists purely as the saved
    // grouping/rect a restore gesture reads. A direct tree write, not an op:
    // this is construction-time normalization (like the constructor's own
    // decode assignment), not a user/engine gesture. A same-keyed entry still
    // present above (a crafted blob that listed an already-floating panel —
    // its `float` no-op'd, leaving the window undrained) is replaced by this
    // record, never duplicated.
    const liveKeys = new Set(live.map((w) => w.key));
    const popouts = [
      ...l.expanded.popouts.filter((w) => !liveKeys.has(w.key)),
      ...live.map((w): PopoutWindowLayout => ({ ...w, dormant: true })),
    ];
    l = { ...l, expanded: { ...l.expanded, popouts } };
    this.#layout = l;
    this.#persist(l);
    this.#pendingNotice = "panels.popoutRestoredFloating";
  }

  /** Flushes a notice queued during construction (currently only
   * `#rehydratePoppedOut`'s reload-restore notice) through `deps.onNotice`.
   * MUST be called from a post-mount hook (e.g. `PanelHost`'s `$effect`),
   * never synchronously alongside construction: `PanelsController` is built
   * in the host's `<script>` body, before the DOM is attached, so calling
   * `deps.onNotice` directly from `#rehydratePoppedOut` would set the a11y
   * live region's (`role="status" aria-live="polite"`) text before its FIRST
   * render — a `polite` region only announces content CHANGES, so text
   * already present at initial paint is silently swallowed by assistive
   * tech. No-op when nothing is queued (the common case: most mounts have no
   * persisted popout to rehydrate). Idempotent: a second call after the
   * first is also a no-op, so a caller need not guard re-invocation.
   * @example
   * ```ts
   * import { PanelsController } from "@shadowcat/module-panels";
   * import type { PanelsControllerDeps } from "@shadowcat/module-panels";
   *
   * declare const deps: PanelsControllerDeps;
   * const controller = new PanelsController(deps);
   * // called from a post-mount effect, e.g. PanelHost's `$effect`
   * controller.flushPendingNotice();
   * ```
   */
  flushPendingNotice(): void {
    if (this.#pendingNotice === null) return;
    const key = this.#pendingNotice;
    this.#pendingNotice = null;
    this.#deps.onNotice?.(key);
  }

  /** The current `PanelLayoutV1` tree — `$state`, so a reactive reader
   * (template/`$derived`) re-runs on every layout-changing `dispatch`.
   * @returns The current layout tree.
   * @example
   * ```ts
   * import { PanelsController } from "@shadowcat/module-panels";
   * import type { PanelsControllerDeps } from "@shadowcat/module-panels";
   *
   * declare const deps: PanelsControllerDeps;
   * const controller = new PanelsController(deps);
   * const layout = controller.layout;
   * ```
   */
  get layout(): PanelLayoutV1 {
    return this.#layout;
  }

  /** Current registrations, gmOnly-filtered for `deps.role`, reactive to the
   * contribution registry (module install/uninstall).
   * @returns The role-filtered `shadowcat.panel` registrations.
   */
  get visibleRegs(): Contribution[] {
    this.#subscribe();
    return regsForRole(this.#deps.contributions.contributionsFor(PANEL_CONTRACT), this.#deps.role);
  }

  /** Per-panel icon/label/badge metadata for every currently-visible
   * registration, keyed by panel id — the shape `EngineAdapter.apply` and the
   * compact switcher both consume for tab/chip chrome.
   * @returns A map of panel id to its `PanelMeta`.
   */
  get metaMap(): ReadonlyMap<string, PanelMeta> {
    const m = new Map<string, PanelMeta>();
    for (const c of this.visibleRegs) if (c.panel) m.set(c.id, c.panel);
    return m;
  }

  /** Currently minimized panel ids, in `layout` order. `#layout` is `$state`,
   * so a caller reading this inside a Svelte `$derived`/template — even
   * through the `PanelsBridge` indirection `bind()` sets up — establishes a
   * reactive dependency on every later layout change, not just the value at
   * bind time (the `panels:chips` live-props requirement).
   * @returns The minimized panel ids, in layout order.
   */
  get minimized(): readonly string[] {
    return this.#layout.expanded.minimized;
  }

  /** Applies a `LayoutOp` — from an engine gesture (`PanelHost`'s `eng.onOp`)
   * or one of the `PanelsApi` methods below — and persists the result.
   * `applyOp` returns the SAME reference on a no-op (e.g. `open` on an
   * already-focused panel); relying on that reference-equality contract here
   * is what keeps a no-op gesture from re-encoding and re-persisting an
   * unchanged tree on every call.
   * @param op The layout op to apply.
   * @example
   * ```ts
   * import { PanelsController } from "@shadowcat/module-panels";
   * import type { PanelsControllerDeps } from "@shadowcat/module-panels";
   *
   * declare const deps: PanelsControllerDeps;
   * const controller = new PanelsController(deps);
   * controller.dispatch({ op: "minimize", id: "chat" });
   * ```
   */
  dispatch(op: LayoutOp): void {
    const prev = this.#layout;
    const next = applyOp(prev, op);
    if (next === prev) return;
    this.#layout = next;
    this.#persist(next);
    this.#deps.onOp?.(op, prev);
  }

  /** Drops any panel id no longer present among current registrations
   * (module uninstalled/renamed since the layout was built), THEN
   * default-places any registration this layout has never recorded —
   * `regsForRole`'s live registry read can (and, per module registration
   * order, routinely does) return more contributions after this controller's
   * own construction than it did when `defaultLayout` first ran, so an id
   * that simply hadn't registered yet at construction time would otherwise
   * sit forever unreachable (no zone, no minimized chip, no compact-switcher
   * tab). Called by `PanelHost`'s registration-change effect on every
   * `visibleRegs` change. Same reference-equality contract as `dispatch`:
   * returns without persisting when neither pass changed anything.
   * @param regs The current, role-filtered registrations to reconcile the
   * layout against.
   * @example
   * ```ts
   * import { PanelsController } from "@shadowcat/module-panels";
   * import type { PanelsControllerDeps } from "@shadowcat/module-panels";
   * import type { Contribution } from "@shadowcat/core";
   *
   * declare const deps: PanelsControllerDeps;
   * declare const regs: readonly Contribution[];
   * const controller = new PanelsController(deps);
   * controller.syncRegistrations(regs);
   * ```
   */
  syncRegistrations(regs: readonly Contribution[]): void {
    const known = new Set(regs.map((c) => c.id));
    let next = prune(this.#layout, known);
    next = placeNewRegistrations(
      next,
      regs.map((c) => ({ id: c.id, placement: c.panel?.defaultPlacement })),
      this.#persistedSource,
    );
    if (next === this.#layout) return;
    this.#layout = next;
    this.#persist(next);
  }

  /** Encodes `l` and hands it to `deps.setPanelLayout` — the sole write path
   * to persisted layout storage.
   * @param l The layout to persist.
   * @example
   * ```
   * // private method; not part of the public API — invoked only from
   * // dispatch/syncRegistrations/#rehydratePoppedOut after a layout change
   * this.#persist(this.#layout);
   * ```
   */
  #persist(l: PanelLayoutV1): void {
    this.#deps.setPanelLayout(encodeLayout(l));
  }

  // --- PanelsApi ---

  /** Surfaces a panel using its OWN `PanelMeta.defaultPlacement` — the same
   * placement `defaultLayout` would have used at first launch — regardless
   * of whether it is currently closed, minimized, or already docked/floating
   * (in the latter two cases `applyOp`'s `open` op is a focus operation:
   * activates its tab, or bumps a floating window to the front).
   * @param id The panel id to open/focus.
   * @example
   * ```ts
   * import { PanelsController } from "@shadowcat/module-panels";
   * import type { PanelsControllerDeps } from "@shadowcat/module-panels";
   *
   * declare const deps: PanelsControllerDeps;
   * const controller = new PanelsController(deps);
   * controller.open("chat");
   * ```
   */
  open(id: string): void {
    const reg = this.visibleRegs.find((c) => c.id === id);
    this.dispatch({ op: "open", id, placement: reg?.panel?.defaultPlacement });
  }

  /** Closes a panel — removes it from whatever zone/floating/minimized
   * location it currently occupies, back to `"closed"` (absent from the
   * layout tree entirely, reopenable only via the topbar launcher).
   * @param id The panel id to close.
   * @example
   * ```ts
   * import { PanelsController } from "@shadowcat/module-panels";
   * import type { PanelsControllerDeps } from "@shadowcat/module-panels";
   *
   * declare const deps: PanelsControllerDeps;
   * const controller = new PanelsController(deps);
   * controller.close("chat");
   * ```
   */
  close(id: string): void {
    this.dispatch({ op: "close", id });
  }

  /** Brings a panel to the foreground via `applyOp`'s `open` op — the
   * layout-tree's own focus operation (activates its tab if docked, or bumps
   * a floating window's z-order). `PanelHost`'s `onOp` callback additionally
   * calls the active engine's `EngineAdapter.focus(id)` for every `"open"`
   * op this dispatches (imperative DOM-level focus, e.g. raising a floating
   * window), so this method's effect is the `dispatch`ed `open` op above PLUS
   * that engine-level focus, not the tree state alone.
   * @param id The panel id to bring to the foreground.
   * @example
   * ```ts
   * import { PanelsController } from "@shadowcat/module-panels";
   * import type { PanelsControllerDeps } from "@shadowcat/module-panels";
   *
   * declare const deps: PanelsControllerDeps;
   * const controller = new PanelsController(deps);
   * controller.focus("chat");
   * ```
   */
  focus(id: string): void {
    this.open(id);
  }

  /** Minimizes an open/docked/floating panel; opens (per its own
   * `defaultPlacement`) a minimized or closed one.
   * @param id The panel id to toggle.
   * @example
   * ```ts
   * import { PanelsController } from "@shadowcat/module-panels";
   * import type { PanelsControllerDeps } from "@shadowcat/module-panels";
   *
   * declare const deps: PanelsControllerDeps;
   * const controller = new PanelsController(deps);
   * controller.toggle("chat");
   * ```
   */
  toggle(id: string): void {
    const loc = locate(this.#layout, id);
    if (loc.where === "minimized" || loc.where === "closed") {
      this.open(id);
    } else {
      this.dispatch({ op: "minimize", id });
    }
  }

  // --- PanelsChipsView ---

  /** Restores a minimized panel (docks it to a new "right" group, mirroring
   * `applyOp`'s `restore` op) — the `panel-dock` chip strip's click handler.
   * @param id The panel id to restore.
   * @example
   * ```ts
   * import { PanelsController } from "@shadowcat/module-panels";
   * import type { PanelsControllerDeps } from "@shadowcat/module-panels";
   *
   * declare const deps: PanelsControllerDeps;
   * const controller = new PanelsController(deps);
   * controller.restore("chat");
   * ```
   */
  restore(id: string): void {
    this.dispatch({ op: "restore", id });
  }
}
