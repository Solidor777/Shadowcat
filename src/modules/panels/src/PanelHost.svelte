<script lang="ts">
  import { untrack, type Component } from "svelte";
  import { getAppContext, sizeClass, Surface } from "@shadowcat/ui-kit";
  import { consoleLogger, type Logger } from "@shadowcat/core";
  import type { EngineAdapter } from "./engine/adapter";
  import { FakeEngine } from "./engine/fake";
  import { PanelsController, type PanelsBridgeLike } from "./controller.svelte";
  import { locate, type LayoutOp, type PanelLayoutV1 } from "./layout/tree";
  import CompactSwitcher from "./CompactSwitcher.svelte";

  /** `engine` defaults to `FakeEngine` (the bespoke-fallback engine); a real
   * docking engine can be injected by a caller. `controller` defaults to a
   * `PanelsController` built from AppContext (persisted layout + gmOnly
   * filtering + `PanelsApi`); tests inject their own to assert against it
   * directly. `logger` mirrors `PanelsBridge`'s pattern — no logger seam
   * exists on AppContext, so this component accepts one as an optional prop
   * and falls back to the production console logger. */
  let {
    engine,
    logger,
    controller,
  }: {
    /** The docking engine to reconcile the layout onto; see this component's own doc. */
    engine?: EngineAdapter;
    /** Diagnostic sink for recoverable failures; see this component's own doc. */
    logger?: Logger;
    /** The layout-owning controller; see this component's own doc. */
    controller?: PanelsController;
  } = $props();
  const log = untrack(() => logger ?? consoleLogger());

  const ctx = getAppContext();
  const t = ctx.t;

  // The a11y live-region text (`panels.moved`), announced once per
  // layout-changing op `dispatch` accepts — see `describeOp` below. `$state`
  // so re-announcing the SAME text still fires (Svelte skips a no-op
  // reassignment only when the VALUE is identical; two consecutive `dock`s
  // to different zones always differ, and even an identical repeat is
  // harmless — a screen reader coalescing an unchanged polite region is
  // expected, not a bug this needs to work around).
  let announce = $state("");

  /** Maps a layout-changing op to its `panels.moved` announcement, or `null`
   * for an op that isn't worth narrating (tab-switch, resize, compact-view
   * switch). Reuses the existing `panels.dockRight`/`dockBottom`/`dockLeft`/
   * `float`/`minimize`/`restore`/`close` keys as the "where" phrase — the
   * same words a sighted user already sees on the chip strip/menu.
   * `"open"` narrates only when `applyOp`'s `"open"` case actually changed
   * placement (surfaced a minimized, closed, or popped-out panel via
   * `placeByPlacement`, per `prevLayout`'s recorded location) rather than
   * merely bumping focus within an already-docked group or an
   * already-floating window — the latter two stay silent, unchanged from
   * every other case here.
   * `PanelsApi.open` is public and reachable outside any control this host
   * renders (`SceneBrowserPanel`'s per-scene configure button,
   * `SheetsController.openDocument`), both routing through
   * `PanelsController.dispatch` like any other op.
   * @param op The layout-changing op `PanelsController.onOp` fired.
   * @param prevLayout The layout as it stood immediately before `op` was
   * applied — the sole way to tell a real "open" placement change from a
   * focus bump, since `ctrl.layout` already reflects the post-op state by
   * the time this runs.
   * @returns The `panels.moved` announcement text, or `null` for an op not
   * worth narrating.
   * @example
   * ```
   * // private function; not part of the public API — invoked only from
   * // the controller's onOp callback below
   * declare const prevLayout: PanelLayoutV1;
   * describeOp({ op: "minimize", id: "chat" }, prevLayout);
   * ```
   */
  function describeOp(op: LayoutOp, prevLayout: PanelLayoutV1): string | null {
    const label = (id: string): string => {
      const meta = ctrl.metaMap.get(id);
      return meta ? t(meta.labelKey) : t("panels.unknownPanel", { id });
    };
    let where: string;
    switch (op.op) {
      case "dock":
        where = t(op.zone === "right" ? "panels.dockRight" : op.zone === "bottom" ? "panels.dockBottom" : "panels.dockLeft");
        break;
      case "float":
        where = t("panels.float");
        break;
      case "minimize":
        where = t("panels.minimize");
        break;
      case "restore":
        where = t("panels.restore");
        break;
      case "close":
        where = t("panels.close");
        break;
      case "popOut":
        where = t("panels.popOut");
        break;
      case "popIn":
        where = t("panels.restore");
        break;
      case "open": {
        const prevWhere = locate(prevLayout, op.id).where;
        // Matches `applyOp`'s ACTUAL condition (docked/floating are the
        // only two branches it special-cases as a focus bump) rather than
        // enumerating the fallthrough's members — structurally can't miss
        // a future `PanelLocation` variant the way an allow-list of
        // "minimized"/"closed" alone would (it silently omitted
        // "popped-out", also part of the fallthrough).
        if (prevWhere === "docked" || prevWhere === "floating") return null;
        // Placement changed: read where it actually landed from the
        // post-op state (`ctrl.layout`), since `placeByPlacement` maps
        // `DefaultPlacement.kind` onto exactly these three shapes.
        const loc = locate(ctrl.layout, op.id);
        if (loc.where === "docked") {
          where = t(loc.zone === "right" ? "panels.dockRight" : loc.zone === "bottom" ? "panels.dockBottom" : "panels.dockLeft");
        } else if (loc.where === "floating") {
          where = t("panels.float");
        } else if (loc.where === "minimized") {
          where = t("panels.minimize");
        } else {
          return null;
        }
        break;
      }
      default:
        // resizeZone/resizeGroup/activeTab/compactView: not narrated.
        return null;
    }
    return t("panels.moved", { panel: label(op.id), where });
  }

  // `AppContext.panels` is typed as `PanelsApi & PanelsChipsView`, which
  // intentionally omits `bind` (a proxy-rebind affordance, not part of the
  // general contract other callers use). This rests on composition-root
  // convention, not the type system: `Table` is the sole place that
  // constructs the concrete `PanelsBridge` and assigns it to `ctx.panels`,
  // so this is the sole binding site — guarded at runtime rather than cast
  // unchecked, so a violated convention fails loudly here instead of
  // surfacing as a confusing error deeper inside `PanelsController`.
  if (typeof (ctx.panels as Partial<PanelsBridgeLike>)?.bind !== "function") {
    throw new Error(
      "PanelHost expects AppContext.panels to be a PanelsBridgeLike (missing .bind) — check the composition-root binding in Table.svelte",
    );
  }
  const bridge: PanelsBridgeLike = ctx.panels as unknown as PanelsBridgeLike;

  const ctrl = untrack(
    () =>
      controller ??
      new PanelsController({
        contributions: ctx.contributions,
        role: ctx.role,
        getPanelLayout: () => ctx.uiState.getPanelLayout(),
        setPanelLayout: (blob) => ctx.uiState.setPanelLayout(blob),
        bridge,
        logger: log,
        // The controller already logs a reset via `logger.warn`; `onReset` is
        // the seam a visible toast (e.g. a statusbar live region) hangs off
        // once that surface exists — a no-op until then.
        onReset: () => {},
        onNotice: (key) => {
          announce = t(key);
        },
        onOp: (op, prev) => {
          const text = describeOp(op, prev);
          if (text !== null) announce = text;
        },
      }),
  );

  // Flushes any notice `PanelsController` queued during its own construction
  // (currently only the reload-restore notice — see `flushPendingNotice`'s
  // doc comment for why this can't fire from the constructor's `onNotice`
  // callback itself). Reads no reactive state, so this `$effect` runs
  // exactly once, after first mount — the live region below is guaranteed
  // to have already painted its EMPTY initial value by the time this can
  // possibly change it, which is what makes the change announced at all.
  $effect(() => {
    ctrl.flushPendingNotice();
  });

  // gmOnly filtering happens once, in the controller — every downstream
  // consumer (switcher, chips, engine) sees only the already-filtered set.
  const visibleRegs = $derived(ctrl.visibleRegs);
  const metaMap = $derived(ctrl.metaMap);
  const layout = $derived(ctrl.layout);

  const eng: EngineAdapter = untrack(() => engine ?? new FakeEngine());

  let hostEl: HTMLElement;
  let stageEl: HTMLElement;
  let stagingEl: HTMLElement;
  let compactStageEl: HTMLElement;
  // The stage's engine-adopted parent (`FakeEngine`'s center-well, or
  // dockview's stage-panel content container) — captured once, right after
  // `eng.init()` synchronously appends `stageEl` there. `$state` so the
  // adoption effect below reactively re-runs once this transitions from
  // null (mount-before-init race) to set, rather than depending on
  // declaration/scheduling order between the two effects — this
  // async-completion guard keys on object identity, not a mode string.
  let stageHomeEl = $state<HTMLElement | null>(null);
  const slotEls = new Map<string, HTMLElement>();
  // Bumped only by a boundary's reload affordance — the sole sanctioned
  // remount path; ordinary layout ops never touch it.
  let remountKeys = $state<Map<string, number>>(new Map());

  // Reactively syncs the controller's layout against `visibleRegs` whenever a
  // registration is added/removed — BEFORE the reconcile effect below can
  // hand a stale id to `eng.apply`/`slotFor`. `syncRegistrations` both prunes
  // ids no longer registered AND default-places any id this layout has never
  // recorded (module registration order does not guarantee every panel
  // module has contributed by the time this controller was constructed).
  // Reads AND conditionally writes the controller's layout state; that read
  // must stay untracked here (mirroring the untracked `remountKeys` read
  // below), or this effect would also depend on the very state it writes and
  // self-retrigger every pass (an infinite update loop) — it relies solely
  // on `syncRegistrations`'s same-reference no-op contract to skip
  // persisting when nothing actually changed. Depends only on `visibleRegs`,
  // so this effect's own writes cannot re-trigger itself (no prune/apply
  // race).
  $effect(() => {
    const regs = visibleRegs;
    const known = new Set(regs.map((c) => c.id));
    untrack(() => ctrl.syncRegistrations(regs));
    const keys = untrack(() => remountKeys);
    let changed = false;
    const nm = new Map(keys);
    for (const id of [...nm.keys()]) {
      if (!known.has(id)) {
        nm.delete(id);
        changed = true;
      }
    }
    if (changed) remountKeys = nm;
  });

  /** Returns a slot element CompactSwitcher adopted back to the staging
   * container — passed to `CompactSwitcher` as its `release` prop, so it (not
   * this host) decides when an adopted slot's ownership needs reclaiming (a
   * switch to a different tab, leaving compact presentation, or unmount).
   * @param el The slot element to return to staging.
   * @example
   * ```
   * // private function; not part of the public API — passed to
   * // CompactSwitcher as its `release` prop below
   * declare const slotEl: HTMLElement;
   * releaseToStaging(slotEl);
   * ```
   */
  function releaseToStaging(el: HTMLElement): void {
    if (stagingEl && el.parentElement !== stagingEl) stagingEl.appendChild(el);
  }

  /** Never throws on an unknown/removed id: a bug upstream then degrades to a
   * missing panel, not a dead reactive graph.
   * @param id The panel id to resolve a slot for.
   * @returns The registered slot element for `id`, or a detached placeholder
   * (never appended anywhere visible) if none is registered.
   * @example
   * ```
   * // private function; not part of the public API — passed to the engine's
   * // `init()` and to CompactSwitcher/registerSlot below
   * slotFor("chat");
   * ```
   */
  function slotFor(id: string): HTMLElement {
    const el = slotEls.get(id);
    if (el) return el;
    log.warn(`panel-host: no slot registered for panel "${id}"; returning a detached placeholder`);
    return document.createElement("div");
  }

  /** Svelte action registering an `{#each}` iteration's slot element under its
   * panel id — the per-id ref binding `bind:this` cannot express here.
   * @param node The slot element this `{#each}` iteration mounted.
   * @param id The panel id this iteration corresponds to.
   * @returns A Svelte action lifecycle object; `destroy()` unregisters the
   * slot only if it is still the currently-registered one for `id`.
   * @example
   * ```
   * // not exported; used only via `use:registerSlot={id}` in this component's
   * // own template — shown here as a direct call for typechecking purposes
   * declare const node: HTMLElement;
   * const action = registerSlot(node, "chat");
   * action.destroy();
   * ```
   */
  function registerSlot(node: HTMLElement, id: string): {
    /** Unregisters this slot, per this function's own doc. */
    destroy(): void;
  } {
    slotEls.set(id, node);
    return {
      destroy() {
        if (slotEls.get(id) === node) slotEls.delete(id);
      },
    };
  }

  /** Discards and re-mounts a fresh instance of panel `id` — the sole
   * sanctioned remount path (see the `{#key}` in the template below);
   * ordinary layout ops never touch `remountKeys`.
   * @param id The crashed panel's id, from its own `svelte:boundary` reload button.
   * @example
   * ```
   * // private function; not part of the public API — invoked only from the
   * // crashed-panel boundary's reload button in this component's template
   * bumpRemount("chat");
   * ```
   */
  function bumpRemount(id: string): void {
    const m = new Map(remountKeys);
    m.set(id, (m.get(id) ?? 0) + 1);
    remountKeys = m;
  }

  // Engine init/destroy: runs once at mount (reads only non-reactive `let`
  // bindings, so this effect never re-runs on its own).
  $effect(() => {
    if (!hostEl || !stageEl) return;
    eng.init(hostEl, slotFor, stageEl);
    stageHomeEl = stageEl.parentElement;
    const unsubOp = eng.onOp((op) => {
      ctrl.dispatch(op);
    });
    const unsubNotice = eng.onNotice?.((key) => {
      announce = t(key);
    });
    return () => {
      unsubOp();
      unsubNotice?.();
      eng.destroy();
      stageHomeEl = null;
    };
  });

  // The always-present canvas Surface inside `.stage` must never unmount
  // ({#key}/{#if} on it are forbidden — CSS-hide + appendChild moves only).
  // `.engine-host` is `hidden` while compact, so the engine's own adoption
  // location for `stageEl` sits inside a `[hidden]` ancestor in that mode;
  // this effect relocates `stageEl` itself instead: into the persistent
  // `.compact-stage` well while compact, back into the engine's own adopted
  // location (`stageHomeEl`) while expanded. Guarded by comparing actual DOM
  // parent identity (not just the mode string), so a same-tick race with the
  // engine-init effect above can't strand it unparented.
  $effect(() => {
    if (!stageEl) return;
    if (sizeClass() === "compact") {
      if (compactStageEl && stageEl.parentElement !== compactStageEl) {
        compactStageEl.appendChild(stageEl);
      }
    } else if (stageHomeEl && stageEl.parentElement !== stageHomeEl) {
      stageHomeEl.appendChild(stageEl);
    }
  });

  // Reconcile the engine only while expanded is the active presentation —
  // CompactSwitcher governs slot adoption while compact. Re-running this
  // effect on every flip back to expanded reclaims any slot CompactSwitcher
  // adopted meanwhile (`apply` is idempotent/full-rebuild).
  $effect(() => {
    if (sizeClass() !== "expanded") return;
    eng.apply(layout.expanded, metaMap);
    if (!stagingEl) return;
    for (const id of layout.expanded.minimized) {
      const slot = slotEls.get(id);
      if (slot && slot.parentElement !== stagingEl) stagingEl.appendChild(slot);
    }
  });
</script>

<div class="panel-host">
  <!-- Every visible registration's component mounts here EXACTLY ONCE, for as
       long as the registration exists. Hosts (engine groups, the compact
       active view, ...) adopt these elements via `appendChild`; this
       container only ever hides via CSS, never `{#if}`. The `{#each}` below
       creates each iteration's slot in `visibleRegs` order, which is why that
       order is this staging container's render/DOM order — a fact about
       staging-slot CREATION sequence only; it carries no z-order or visual
       stacking meaning for any host that later adopts a slot elsewhere. -->
  <div class="staging" bind:this={stagingEl}>
    {#each visibleRegs as c (c.id)}
      {@const Comp = c.component as Component<Record<string, unknown>>}
      <div class="panel-slot" data-panel={c.id} use:registerSlot={c.id}>
        {#key remountKeys.get(c.id) ?? 0}
          <svelte:boundary onerror={(e) => log.error(`panel "${c.id}" crashed`, e)}>
            <Comp {...(c.props ?? {})} />
            {#snippet failed(_error, _reset)}
              <div class="crashed" data-testid="crashed-{c.id}">
                <span>{t("panels.crashed")}</span>
                <button
                  type="button"
                  data-testid="reload-{c.id}"
                  onclick={() => {
                    // The {#key} bump below is the sole sanctioned remount
                    // path: it discards + re-mounts a fresh instance and, in
                    // doing so, tears down this boundary along with it —
                    // calling the boundary's own `reset()` too would mount a
                    // second, immediately-discarded instance first.
                    bumpRemount(c.id);
                  }}
                >{t("panels.reload")}</button>
              </div>
            {/snippet}
          </svelte:boundary>
        {/key}
      </div>
    {/each}
  </div>

  <!-- The always-present canvas/stage content (module-stage's contribution
       into core-ui's singleton `shadowcat.surface:stage`) lives inside the
       engine's reserved stage well — never a draggable/closable panel; see
       `STAGE_ID`/`classifyDrop`'s stage vetoes. -->
  <div class="stage" bind:this={stageEl}>
    <Surface contract="shadowcat.surface:stage" />
  </div>

  <div class="engine-host" bind:this={hostEl} hidden={sizeClass() !== "expanded"}></div>

  <!-- Persistent compact-mode home for `stageEl` (see the adoption effect
       above) — fills the compact layout behind the switcher so the canvas
       stays outside any `[hidden]` ancestor instead of trapped inside
       `.engine-host`. -->
  <div class="compact-stage" bind:this={compactStageEl} hidden={sizeClass() !== "compact"}></div>

  <CompactSwitcher
    order={layout.compact.order}
    activeView={layout.compact.activeView}
    meta={metaMap}
    {slotFor}
    release={releaseToStaging}
    onSwitch={(id) => ctrl.dispatch({ op: "compactView", id })}
  />

  <!-- Narrates every layout-changing op (drag OR menu-driven — both funnel
       through `ctrl.dispatch`, see `describeOp`) — sighted-only chip/menu
       affordances otherwise give a screen-reader user no signal a panel moved. -->
  <div class="sr-only" role="status" aria-live="polite">{announce}</div>
</div>

<style lang="scss">
  .panel-host {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    position: relative;
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    padding: 0;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
  }
  .staging {
    display: none;
  }
  .stage {
    height: 100%;
    width: 100%;
  }
  .engine-host {
    flex: 1;
    min-height: 0;
  }
  .compact-stage {
    position: absolute;
    inset: 0;
  }
  // CompactSwitcher is rendered as an ordinary in-flow child; a positioned
  // sibling with z-index 0 (`.compact-stage`) paints AFTER in-flow,
  // non-positioned boxes in stacking order, which would otherwise cover it.
  // Giving the switcher its own stacking context above `.compact-stage`
  // keeps it visible on top of the canvas layer, matching a docked-panel-
  // over-canvas mobile layout.
  :global(.compact-switcher) {
    position: relative;
    z-index: 1;
  }
  .crashed {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.75rem;
    color: var(--text-muted);
  }
</style>
