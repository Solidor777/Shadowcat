<script lang="ts">
  import { untrack, type Component } from "svelte";
  import { getAppContext, sizeClass, Surface } from "@shadowcat/ui-kit";
  import { consoleLogger, type Logger } from "@shadowcat/core";
  import type { EngineAdapter } from "./engine/adapter";
  import { FakeEngine } from "./engine/fake";
  import { PanelsController, type PanelsBridgeLike } from "./controller.svelte";
  import type { LayoutOp } from "./layout/tree";
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
  }: { engine?: EngineAdapter; logger?: Logger; controller?: PanelsController } = $props();
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
   * `"open"` is NOT narrated as a "move" here even though `applyOp`'s
   * `"open"` case can be a real placement change (surfacing a minimized or
   * closed panel into a docked group, via `placeByPlacement`) rather than a
   * mere focus bump within an already-open group/floating window — no
   * current UI affordance dispatches `"open"` (unreachable from any control
   * this host renders today), so this is dead code pending a future
   * command palette, not a narration bug against any live path. */
  function describeOp(op: LayoutOp): string | null {
    const label = (id: string): string => {
      const meta = ctrl.metaMap.get(id);
      return meta ? t(meta.labelKey) : id;
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
      default:
        // resizeZone/resizeGroup/activeTab/compactView/open: not narrated —
        // see the doc comment above for why "open" specifically is excluded.
        return null;
    }
    return t("panels.moved", { panel: label(op.id), where });
  }

  // `AppContext.panels` is typed as `PanelsApi & PanelsChipsView`, which
  // intentionally omits `bind` (a proxy-rebind affordance, not part of the
  // general contract other callers use). This rests on composition-root
  // convention, not the type system: `Table.svelte` is the sole place that
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
        onOp: (op) => {
          const text = describeOp(op);
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
  // declaration/scheduling order between the two effects (buddy-check
  // finding 1's async-completion guard: identity, not a mode string).
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

  function releaseToStaging(el: HTMLElement): void {
    if (stagingEl && el.parentElement !== stagingEl) stagingEl.appendChild(el);
  }

  // Never throws on an unknown/removed id (containment for finding 1): a bug
  // upstream then degrades to a missing panel, not a dead reactive graph.
  // Returns a detached placeholder that is never appended anywhere visible.
  function slotFor(id: string): HTMLElement {
    const el = slotEls.get(id);
    if (el) return el;
    log.warn(`panel-host: no slot registered for panel "${id}"; returning a detached placeholder`);
    return document.createElement("div");
  }

  /** Svelte action registering an `{#each}` iteration's slot element under its
   * panel id — the per-id ref binding `bind:this` cannot express here. */
  function registerSlot(node: HTMLElement, id: string): { destroy(): void } {
    slotEls.set(id, node);
    return {
      destroy() {
        if (slotEls.get(id) === node) slotEls.delete(id);
      },
    };
  }

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
