<script lang="ts">
  import type { Component } from "svelte";
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "./appContext";

  let {
    contract,
    activeId = null,
    onTabChange,
  }: {
    contract: string;
    activeId?: string | null;
    onTabChange?: (id: string) => void;
  } = $props();

  const ctx = getAppContext();
  const t = ctx.t;

  const subscribe = createSubscriber((update) => ctx.contributions.subscribe(update));
  // gmOnly tabs are host-filtered for non-GM (panels may additionally self-gate).
  const items = $derived.by(() => {
    subscribe();
    return ctx.contributions
      .contributionsFor(contract)
      .filter((c) => !(c.tab?.gmOnly && ctx.role !== "gm"));
  });
  // activeId wins when it names a visible tab; otherwise first visible.
  const active = $derived(items.find((c) => c.id === activeId)?.id ?? items[0]?.id ?? null);

  let collapsed = $state(false);

  function pick(id: string): void {
    if (collapsed) collapsed = false;
    onTabChange?.(id);
  }
  function label(c: (typeof items)[number]): string {
    return c.tab ? t(c.tab.labelKey) : c.id;
  }
</script>

<div class="tabbed" class:collapsed>
  <div class="rail" aria-orientation="vertical" role="tablist">
    <button
      type="button"
      class="rail-btn toggle"
      aria-label={collapsed ? t("sidebar.expand") : t("sidebar.collapse")}
      title={collapsed ? t("sidebar.expand") : t("sidebar.collapse")}
      onclick={() => (collapsed = !collapsed)}
    >{collapsed ? "◂" : "▸"}</button>
    {#each items as c (c.id)}
      <button
        type="button"
        class="rail-btn"
        role="tab"
        aria-selected={c.id === active}
        aria-label={label(c)}
        title={label(c)}
        data-testid="tab-{c.id}"
        onclick={() => pick(c.id)}
      >{c.tab?.icon ?? c.id.slice(0, 1)}</button>
    {/each}
  </div>
  <div class="content" hidden={collapsed}>
    <!-- Every panel stays mounted (state/scroll preserved; GM seed $effects run
         regardless of the active tab, and across collapse/expand): the inactive
         ones are display:none, and collapsing hides the whole content area the
         same non-destructive way — never unmounted. -->
    {#each items as c (c.id)}
      {@const Comp = c.component as Component<Record<string, unknown>>}
      <div class="panel" role="tabpanel" hidden={c.id !== active} data-testid="panel-{c.id}">
        <Comp {...(c.props ?? {})} />
      </div>
    {/each}
  </div>
</div>

<style lang="scss">
  .tabbed {
    display: flex;
    flex-direction: row-reverse; /* rail on the outer (right) edge */
    height: 100%;
    min-height: 0;
  }
  .rail {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    padding: 0.25rem;
    border-left: 1px solid var(--border);
    background: var(--surface-overlay);
  }
  .rail-btn {
    /* Touch target floor (mobile invariant). */
    min-width: 44px;
    min-height: 44px;
    border: none;
    border-radius: 0.375rem;
    background: transparent;
    color: var(--text-primary);
    font-size: 1.25rem;
    cursor: pointer;
    &:hover { background: var(--surface-raised); }
    &[aria-selected="true"] { background: var(--surface-base); }
  }
  .content {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    /* [hidden] must win over the class's `display: flex`; browsers give the
       `hidden` attribute UA-stylesheet display:none, but it's easily beaten by
       an author `display` declaration of equal specificity — pin it explicitly. */
    &[hidden] { display: none; }
  }
  .panel {
    flex: 1;
    min-height: 0;
    overflow: auto;
    &[hidden] { display: none; }
  }
</style>
