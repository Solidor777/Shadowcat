<script lang="ts">
  import { untrack, type Component } from "svelte";
  import { getAppContext, sizeClass, Surface } from "@shadowcat/ui-kit";
  import { consoleLogger, type Logger } from "@shadowcat/core";
  import type { EngineAdapter } from "./engine/adapter";
  import { FakeEngine } from "./engine/fake";
  import { PanelsController, type PanelsBridgeLike } from "./controller.svelte";
  import CompactSwitcher from "./CompactSwitcher.svelte";
  import DockChips from "./DockChips.svelte";

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

  const ctrl = untrack(
    () =>
      controller ??
      new PanelsController({
        contributions: ctx.contributions,
        role: ctx.role,
        getPanelLayout: () => ctx.uiState.getPanelLayout(),
        setPanelLayout: (blob) => ctx.uiState.setPanelLayout(blob),
        // `AppContext.panels` is typed as `PanelsApi & PanelsChipsView`, which
        // intentionally omits `bind` (a proxy-rebind affordance, not part of
        // the general contract other callers use). This cast rests on
        // composition-root convention, not the type system: `Table.svelte` is
        // the sole place that constructs the concrete `PanelsBridge` and
        // assigns it to `ctx.panels`, so this is the sole binding site.
        bridge: ctx.panels as unknown as PanelsBridgeLike,
        logger: log,
        // The controller already logs a reset via `logger.warn`; `onReset` is
        // the seam a visible toast (e.g. a statusbar live region) hangs off
        // once that surface exists — a no-op until then.
        onReset: () => {},
      }),
  );

  // gmOnly filtering happens once, in the controller — every downstream
  // consumer (switcher, chips, engine) sees only the already-filtered set.
  const visibleRegs = $derived(ctrl.visibleRegs);
  const metaMap = $derived(ctrl.metaMap);
  const layout = $derived(ctrl.layout);

  const eng: EngineAdapter = untrack(() => engine ?? new FakeEngine());

  let hostEl: HTMLElement;
  let stageEl: HTMLElement;
  let stagingEl: HTMLElement;
  const slotEls = new Map<string, HTMLElement>();
  // Bumped only by a boundary's reload affordance — the sole sanctioned
  // remount path; ordinary layout ops never touch it.
  let remountKeys = $state<Map<string, number>>(new Map());

  const knownIds = $derived(new Set(visibleRegs.map((c) => c.id)));

  // Reactively prunes the controller's layout (dropping any id no longer
  // among `knownIds`) whenever a registration is added/removed — BEFORE the
  // reconcile effect below can hand a stale id to `eng.apply`/`slotFor`.
  // `syncKnownIds` reads AND conditionally writes the controller's layout
  // state; that read must stay untracked here (mirroring the untracked
  // `remountKeys` read below), or this effect would also depend on the very
  // state it writes and self-retrigger every pass (an infinite update loop) —
  // it relies solely on `prune`'s same-reference no-op contract to skip
  // persisting when nothing was actually dropped. Depends only on
  // `knownIds`, so this effect's own writes cannot re-trigger itself (no
  // prune/apply race).
  $effect(() => {
    const known = knownIds;
    untrack(() => ctrl.syncKnownIds(known));
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
    const unsubOp = eng.onOp((op) => {
      ctrl.dispatch(op);
    });
    return () => {
      unsubOp();
      eng.destroy();
    };
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
          <svelte:boundary onerror={() => {}}>
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

  <div class="dock-chips-host" hidden={sizeClass() !== "expanded"}>
    <DockChips
      minimized={layout.expanded.minimized}
      meta={metaMap}
      onRestore={(id) => ctrl.dispatch({ op: "restore", id })}
    />
  </div>

  <CompactSwitcher
    order={layout.compact.order}
    activeView={layout.compact.activeView}
    meta={metaMap}
    {slotFor}
    release={releaseToStaging}
    onSwitch={(id) => ctrl.dispatch({ op: "compactView", id })}
  />
</div>

<style lang="scss">
  .panel-host {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
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
  .crashed {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.75rem;
    color: var(--text-muted);
  }
</style>
