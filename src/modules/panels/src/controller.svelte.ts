// The layout-owning controller for the M12a panel-manager host. Bridges every
// layout-changing input — engine gestures (drag/dock/float/minimize) AND the
// imperative `PanelsApi` any module can invoke via `ctx.panels` — onto the pure
// `applyOp` reducer, then persists the result. `PanelHost` owns the DOM/engine
// adapter and the compact/expanded presentation switch; this class owns
// nothing but layout state, registration visibility, and the persistence
// round-trip, so it is usable and testable without mounting any component.
import { PANEL_CONTRACT, type Contribution, type ContributionRegistry, type Logger, type PanelMeta } from "@shadowcat/core";
import type { WorldRole } from "@shadowcat/types";
import type { PanelsApi } from "@shadowcat/ui-kit";
import { createSubscriber } from "svelte/reactivity";
import { applyOp, defaultLayout, locate, prune, type LayoutOp, type PanelLayoutV1 } from "./layout/tree";
import { decodeLayout, encodeLayout } from "./layout/persist";

/** gmOnly filtering is ADVISORY ONLY: it hides a panel from this client's own
 * UI, exactly like `PanelHost`'s prior inline filter. It is not a security
 * boundary — the server remains the sole authority over any data a gmOnly
 * panel displays or writes. */
export function regsForRole(regs: readonly Contribution[], role: WorldRole): Contribution[] {
  return regs.filter((c) => !(c.panel?.gmOnly && role !== "gm"));
}

/** The narrow shape this controller needs from the shell's `PanelsBridge` —
 * `bind` is not part of `PanelsApi` itself (a proxy-rebind affordance, not
 * something a terminal implementation like this controller needs to expose),
 * so callers hand it in separately from the `PanelsApi` methods below. */
export interface PanelsBridgeLike {
  bind(impl: PanelsApi): void;
}

export interface PanelsControllerDeps {
  contributions: ContributionRegistry;
  role: WorldRole;
  getPanelLayout: () => unknown;
  setPanelLayout: (blob: unknown) => void;
  bridge: PanelsBridgeLike;
  logger: Logger;
  /** Fired with the `panels.layoutReset` i18n key when a persisted layout blob
   * was rejected and the default was substituted in its place; the caller
   * resolves + surfaces it (e.g. a statusbar live-region toast). */
  onReset?: (key: string) => void;
}

const EMPTY_LAYOUT: PanelLayoutV1 = {
  version: 1,
  expanded: { zones: { right: { groups: [], size: 320 }, bottom: { groups: [], size: 240 }, left: { groups: [], size: 320 } }, floating: [], minimized: [] },
  compact: { activeView: null, order: [] },
};

export class PanelsController implements PanelsApi {
  #deps: PanelsControllerDeps;
  #subscribe: () => void;
  #layout = $state<PanelLayoutV1>(EMPTY_LAYOUT);

  constructor(deps: PanelsControllerDeps) {
    this.#deps = deps;
    this.#subscribe = createSubscriber((update) => deps.contributions.subscribe(update));

    const regs = regsForRole(deps.contributions.contributionsFor(PANEL_CONTRACT), deps.role);
    const known = new Set(regs.map((c) => c.id));
    const buildDefault = () => defaultLayout(regs.map((c) => ({ id: c.id, placement: c.panel?.defaultPlacement })));
    const { layout, reset } = decodeLayout(deps.getPanelLayout(), known, buildDefault);
    this.#layout = layout;

    if (reset) {
      this.#persist(layout);
      deps.logger.warn("panels: persisted layout blob was invalid; reset to default");
      deps.onReset?.("panels.layoutReset");
    }

    deps.bridge.bind(this);
  }

  get layout(): PanelLayoutV1 {
    return this.#layout;
  }

  /** Current registrations, gmOnly-filtered for `deps.role`, reactive to the
   * contribution registry (module install/uninstall). */
  get visibleRegs(): Contribution[] {
    this.#subscribe();
    return regsForRole(this.#deps.contributions.contributionsFor(PANEL_CONTRACT), this.#deps.role);
  }

  get metaMap(): ReadonlyMap<string, PanelMeta> {
    const m = new Map<string, PanelMeta>();
    for (const c of this.visibleRegs) if (c.panel) m.set(c.id, c.panel);
    return m;
  }

  /** Applies a `LayoutOp` — from an engine gesture (`PanelHost`'s `eng.onOp`)
   * or one of the `PanelsApi` methods below — and persists the result.
   * `applyOp` returns the SAME reference on a no-op (e.g. `open` on an
   * already-focused panel); relying on that reference-equality contract here
   * is what keeps a no-op gesture from re-encoding and re-persisting an
   * unchanged tree on every call. */
  dispatch(op: LayoutOp): void {
    const next = applyOp(this.#layout, op);
    if (next === this.#layout) return;
    this.#layout = next;
    this.#persist(next);
  }

  /** Drops any panel id no longer present among current registrations
   * (module uninstalled/renamed since the layout was built) — called by
   * `PanelHost`'s registration-change effect. Same reference-equality
   * contract as `dispatch`: `prune` returns the SAME reference when nothing
   * was dropped, so an unrelated registry notification never re-persists. */
  syncKnownIds(known: ReadonlySet<string>): void {
    const next = prune(this.#layout, known);
    if (next === this.#layout) return;
    this.#layout = next;
    this.#persist(next);
  }

  #persist(l: PanelLayoutV1): void {
    this.#deps.setPanelLayout(encodeLayout(l));
  }

  // --- PanelsApi ---

  /** Surfaces a panel using its OWN `PanelMeta.defaultPlacement` — the same
   * placement `defaultLayout` would have used at first launch — regardless
   * of whether it is currently closed, minimized, or already docked/floating
   * (in the latter two cases `applyOp`'s `open` op is a focus operation:
   * activates its tab, or bumps a floating window to the front). */
  open(id: string): void {
    const reg = this.visibleRegs.find((c) => c.id === id);
    this.dispatch({ op: "open", id, placement: reg?.panel?.defaultPlacement });
  }

  close(id: string): void {
    this.dispatch({ op: "close", id });
  }

  /** Bring a panel to the foreground. `applyOp`'s `open` op already IS the
   * layout-tree's focus operation (activates its tab if docked, or bumps a
   * floating window's z-order); imperative DOM-level focus (scrolling a
   * floating window into view, etc.) is the engine adapter's `focus(id)`,
   * which `PanelHost` invokes directly against its own engine instance. */
  focus(id: string): void {
    this.open(id);
  }

  /** Minimizes an open/docked/floating panel; opens (per its own
   * `defaultPlacement`) a minimized or closed one. */
  toggle(id: string): void {
    const loc = locate(this.#layout, id);
    if (loc.where === "minimized" || loc.where === "closed") {
      this.open(id);
    } else {
      this.dispatch({ op: "minimize", id });
    }
  }
}
