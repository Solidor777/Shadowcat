<script lang="ts">
  import { onMount } from "svelte";
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext, sizeClass } from "@shadowcat/ui-kit";
  import type { WireDocument, ResourceRegistryEngine, Resource } from "@shadowcat/core";
  import type { TurnBadge } from "./turnBadge";
  import { rowsFor, moveInOrder, type Row } from "./model";
  import { createReorder } from "./reorder";
  import CombatHeader from "./CombatHeader.svelte";
  import CombatantRow from "./CombatantRow.svelte";
  import AddCombatants from "./AddCombatants.svelte";

  /** CombatTrackerPanel props. */
  interface Props {
    /** The badge instance bound on mount — passed as a contribution prop so the same instance
     * the panel-tab chrome reads is the one this panel binds identity to. */
    badge: TurnBadge;
  }
  const { badge }: Props = $props();

  /** Shape of `CombatEngine`'s `order` field, narrowed for the reorder helpers. */
  type CombatOrderShape = {
    /** The combat's turn-order sequence of combatant document ids. */
    order: string[];
  };
  /** Shape of `CombatEngine`'s `turn` field, narrowed for the current-row highlight. */
  type CombatTurnShape = {
    /** The combatant id whose turn is current, or `null` outside an active turn. */
    turn: string | null;
  };

  const ctx = getAppContext();

  // Reactive bridge: every reactive read below calls subscribe() so it re-resolves once the
  // resync stream populates the store after mount (the GameSettingsPanel/ConditionsPanel
  // pattern — a plain document-store read registers no dependency in Svelte 5's runes system).
  const subscribeDocs = createSubscriber((update) => ctx.documents.subscribe(update));
  const subscribeResolved = createSubscriber((update) => ctx.combat.subscribe(update));

  onMount(() => {
    badge.bind(
      (combatantId) => ctx.documents.get(combatantId)?.owner === ctx.selfId && ctx.role !== "gm",
      () => ctx.notify(ctx.t("combatTracker.yourTurn"), "info"),
    );
  });

  const combats = $derived.by((): WireDocument[] => {
    subscribeDocs();
    return ctx.viewedSceneId ? ctx.combat.combatsFor(ctx.viewedSceneId) : [];
  });

  let selectedId = $state<string | null>(null);

  // The active combat, else the first, is the default selection — relying on CombatApi.combatsFor's
  // own documented active-first ordering rather than re-deriving it here; a manual pick persists
  // until the combat set changes shape again (mirrors GameSettingsPanel's scene-selection pattern).
  const selectedCombat = $derived.by((): WireDocument | undefined => combats.find((c) => c.id === selectedId) ?? combats[0]);

  const rows = $derived.by((): Row[] => {
    subscribeDocs();
    subscribeResolved();
    if (!selectedCombat) return [];
    return rowsFor(ctx.combat.combatants(selectedCombat.id), ctx.combat.resolved);
  });

  const registry = $derived.by((): [string, Resource][] => {
    subscribeDocs();
    const doc = ctx.documents.query("resource-registry")[0];
    const eng = doc?.engine as ResourceRegistryEngine | undefined;
    return Object.entries(eng?.resources ?? {}).sort(([, a], [, b]) => a.order - b.order);
  });

  const can = $derived.by(() => (selectedCombat ? ctx.combat.canAct(selectedCombat.id) : null));
  const compact = $derived(sizeClass() === "compact");

  let busy = $state(false);
  let notation = $state("1d20");

  /** Runs `fn` under the panel's busy flag, surfacing a rejection through `ctx.notify`. Passed
   * down to the header/rows so every intent call shares one busy gate.
   * @param fn The async intent call to run.
   * @example
   * ```
   * // private function; not part of the public API — invoked from every intent-dispatching
   * // control this panel and its children own
   * void run(async () => {});
   * ```
   */
  async function run(fn: () => Promise<void>): Promise<void> {
    busy = true;
    try {
      await fn();
    } catch (e) {
      ctx.notify(e instanceof Error ? e.message : String(e), "warning");
    } finally {
      busy = false;
    }
  }

  /**
   * Creates a combat on the currently viewed scene. A no-op when no scene is viewed.
   * @example
   * ```
   * // private function; not part of the public API — invoked from the "Create" button
   * create();
   * ```
   */
  function create(): void {
    const sceneId = ctx.viewedSceneId;
    if (!sceneId) return;
    void run(async () => {
      ctx.combat.createCombat(sceneId);
    });
  }

  // Reorder: a pointer drag over the row elements, plus Alt+ArrowUp/Down on a focused row. Both
  // paths compute the same `moveInOrder` result and dispatch ONE `reorder` call. Row elements are
  // tracked by id through a `use:` action rather than `bind:this` on an array index — Svelte 5
  // treats `bind:this={arr[i]}` inside a keyed `{#each}` as a non-reactive binding, since the
  // array index is not itself a stable reactive target as rows reorder.
  const rowEls = new Map<string, HTMLElement>();
  /** `use:` action registering `node` under `id` for the reorder drag's geometry lookup.
   * @param node The row's root element.
   * @param id The row's combatant document id.
   * @returns A teardown removing the registration on unmount.
   * @example
   * ```svelte
   * <div use:trackRow={row.doc.id}></div>
   * ```
   */
  function trackRow(node: HTMLElement, id: string): {
    /** Removes `id`'s registration on unmount. */
    destroy: () => void;
  } {
    rowEls.set(id, node);
    return { destroy: () => rowEls.delete(id) };
  }
  const reorder = createReorder(() => rows.map((r) => rowEls.get(r.doc.id)?.getBoundingClientRect() ?? new DOMRect()));

  /**
   * Computes the reordered turn sequence via `moveInOrder` and dispatches the one `reorder`
   * intent both the pointer-drag and keyboard paths share. A no-op with no selected combat or
   * without edit capability.
   * @param from The row's index before the move.
   * @param to The row's index after the move.
   * @example
   * ```
   * // private function; not part of the public API — invoked from onDragStart/onRowKeydown
   * dispatchReorder(0, 2);
   * ```
   */
  function dispatchReorder(from: number, to: number): void {
    if (!selectedCombat || !can?.edit) return;
    const engine = selectedCombat.engine as CombatOrderShape;
    ctx.combat.reorder(selectedCombat.id, moveInOrder(engine.order, from, to));
  }

  /**
   * Begins a pointer-drag reorder on the row at `index`, tracking pointer move/up on `window`
   * until release, at which point the computed move (if any) dispatches via
   * {@link dispatchReorder}. A no-op without edit capability.
   * @param index The dragged row's index.
   * @param ev The originating `pointerdown` event.
   * @example
   * ```
   * // private function; not part of the public API — invoked from a row's pointerdown handler
   * declare const event: PointerEvent;
   * onDragStart(0, event);
   * ```
   */
  function onDragStart(index: number, ev: PointerEvent): void {
    if (!can?.edit) return;
    reorder.beginDrag(index, ev);
    const onMove = (e: PointerEvent) => reorder.move(e);
    const onUp = (e: PointerEvent) => {
      const move = reorder.end(e);
      if (move) dispatchReorder(move.from, move.to);
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  }

  /**
   * Alt+ArrowUp/Down keyboard reorder on the row at `index`, dispatching the same
   * {@link dispatchReorder} the pointer-drag path uses. A no-op without edit capability, without
   * the Alt modifier, or at either end of the order.
   * @param index The focused row's index.
   * @param ev The originating `keydown` event.
   * @example
   * ```
   * // private function; not part of the public API — invoked from a row's keydown handler
   * declare const event: KeyboardEvent;
   * onRowKeydown(0, event);
   * ```
   */
  function onRowKeydown(index: number, ev: KeyboardEvent): void {
    if (!can?.edit || !ev.altKey) return;
    if (ev.key === "ArrowUp" && index > 0) {
      ev.preventDefault();
      dispatchReorder(index, index - 1);
    } else if (ev.key === "ArrowDown" && index < rows.length - 1) {
      ev.preventDefault();
      dispatchReorder(index, index + 1);
    }
  }
</script>

<section class="combat-tracker" class:compact aria-label={ctx.t("combatTracker.title")}>
  <h2>{ctx.t("combatTracker.title")}</h2>

  {#if combats.length > 1}
    <label>
      {ctx.t("combatTracker.pick")}
      <select aria-label="combatTracker.pick" value={selectedCombat?.id}
        onchange={(e) => (selectedId = (e.currentTarget as HTMLSelectElement).value)}>
        {#each combats as c (c.id)}
          <option value={c.id}>{c.name ?? c.id}</option>
        {/each}
      </select>
    </label>
  {/if}

  {#if selectedCombat && can}
    <p data-testid="combat-tracker:selected" hidden>{selectedCombat.id}</p>
    <CombatHeader combat={selectedCombat} {rows} {busy} {run} {compact} bind:notation />
    <div class="rows" class:compact>
      {#each rows as row, i (row.doc.id)}
        <div class="row-wrap" class:stacked={compact} use:trackRow={row.doc.id} onkeydown={(e) => onRowKeydown(i, e)} role="presentation">
          <CombatantRow {row} combatId={selectedCombat.id} {registry} isTurn={(selectedCombat.engine as CombatTurnShape).turn === row.doc.id} {can} {busy} {run} {notation} {onDragStart} index={i} {compact} />
        </div>
      {/each}
    </div>
    <AddCombatants combatId={selectedCombat.id} {rows} />
  {:else if ctx.role === "gm"}
    <p>{ctx.t("combatTracker.noCombat")}</p>
    <button type="button" data-testid="combat-tracker:create" disabled={busy} onclick={create}>{ctx.t("combatTracker.create")}</button>
  {:else}
    <p>{ctx.t("combatTracker.noCombatPlayer")}</p>
  {/if}
  {#if busy}<span aria-hidden="true">{ctx.t("combatTracker.busy")}</span>{/if}
</section>

<style lang="scss">
  .combat-tracker {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    color: var(--text);

    button,
    select {
      min-height: var(--input-height, 32px);
    }

    &.compact {
      button,
      select {
        min-height: 44px;
      }
    }
  }

  .rows {
    display: grid;
    gap: var(--space-1);

    &.compact {
      grid-template-columns: 1fr;
    }
  }

  .row-wrap.stacked {
    display: grid;
    grid-template-columns: 1fr;
  }
</style>
