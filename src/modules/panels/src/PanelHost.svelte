<script lang="ts">
  import { untrack, type Component } from "svelte";
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext, sizeClass } from "@shadowcat/ui-kit";
  import { PANEL_CONTRACT, type PanelMeta } from "@shadowcat/core";
  import { applyOp, defaultLayout, type PanelLayoutV1 } from "./layout/tree";
  import type { EngineAdapter } from "./engine/adapter";
  import { FakeEngine } from "./engine/fake";
  import CompactSwitcher from "./CompactSwitcher.svelte";
  import DockChips from "./DockChips.svelte";

  /** `engine` defaults to `FakeEngine` (the bespoke-fallback engine); a real
   * docking engine can be injected by a caller that owns persisted layout
   * state. `layout` here is derived inline from the current registrations —
   * the seam a layout-owning controller replaces is this `$state` init plus
   * the `applyOp` reducer calls below, without touching props or markup. */
  let { engine }: { engine?: EngineAdapter } = $props();

  const ctx = getAppContext();
  const t = ctx.t;

  const subscribe = createSubscriber((update) => ctx.contributions.subscribe(update));

  // gmOnly filtering happens here ONCE — every downstream consumer (layout,
  // switcher, chips, engine) sees only the already-filtered set.
  const visibleRegs = $derived.by(() => {
    subscribe();
    return ctx.contributions
      .contributionsFor(PANEL_CONTRACT)
      .filter((c) => !(c.panel?.gmOnly && ctx.role !== "gm"));
  });

  const metaMap = $derived.by(() => {
    const m = new Map<string, PanelMeta>();
    for (const c of visibleRegs) if (c.panel) m.set(c.id, c.panel);
    return m;
  });

  // Seeds initial layout from the registrations present at mount. A one-time
  // read by design — the seam a layout-owning controller replaces (see the
  // `engine`/`layout` doc comment above) — so it is explicitly untracked
  // rather than reactively re-deriving on every registry change.
  let layout = $state<PanelLayoutV1>(
    untrack(() =>
      defaultLayout(visibleRegs.map((c) => ({ id: c.id, placement: c.panel?.defaultPlacement }))),
    ),
  );

  const eng: EngineAdapter = untrack(() => engine ?? new FakeEngine());

  let hostEl: HTMLElement;
  let stageEl: HTMLElement;
  let stagingEl: HTMLElement;
  const slotEls = new Map<string, HTMLElement>();
  // Bumped only by a boundary's reload affordance — the sole sanctioned
  // remount path; ordinary layout ops never touch it.
  let remountKeys = $state<Map<string, number>>(new Map());

  function slotFor(id: string): HTMLElement {
    const el = slotEls.get(id);
    if (!el) throw new Error(`panel-host: no slot registered for panel "${id}"`);
    return el;
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
      layout = applyOp(layout, op);
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
       container only ever hides via CSS, never `{#if}`. -->
  <div class="staging" bind:this={stagingEl}>
    {#each visibleRegs as c (c.id)}
      {@const Comp = c.component as Component<Record<string, unknown>>}
      <div class="panel-slot" data-panel={c.id} use:registerSlot={c.id}>
        {#key remountKeys.get(c.id) ?? 0}
          <svelte:boundary onerror={() => {}}>
            <Comp {...(c.props ?? {})} />
            {#snippet failed(_error, reset)}
              <div class="crashed" data-testid="crashed-{c.id}">
                <span>{t("panels.crashed")}</span>
                <button
                  type="button"
                  data-testid="reload-{c.id}"
                  onclick={() => {
                    bumpRemount(c.id);
                    reset();
                  }}
                >{t("panels.reload")}</button>
              </div>
            {/snippet}
          </svelte:boundary>
        {/key}
      </div>
    {/each}
  </div>

  <div class="stage" bind:this={stageEl}></div>

  <div class="engine-host" bind:this={hostEl} hidden={sizeClass() !== "expanded"}></div>

  <div class="dock-chips-host" hidden={sizeClass() !== "expanded"}>
    <DockChips
      minimized={layout.expanded.minimized}
      meta={metaMap}
      onRestore={(id) => (layout = applyOp(layout, { op: "restore", id }))}
    />
  </div>

  <CompactSwitcher
    order={layout.compact.order}
    activeView={layout.compact.activeView}
    meta={metaMap}
    {slotFor}
    onSwitch={(id) => (layout = applyOp(layout, { op: "compactView", id }))}
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
