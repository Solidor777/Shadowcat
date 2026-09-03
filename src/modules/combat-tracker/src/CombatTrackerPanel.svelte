<script lang="ts">
  import { onMount } from "svelte";
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import type { WireDocument, ResourceRegistryEngine, Resource } from "@shadowcat/core";
  import type { TurnBadge } from "./turnBadge";
  import { rowsFor, moveInOrder, type Row } from "./model";
  import { createReorder } from "./reorder";
  import CombatHeader from "./CombatHeader.svelte";
  import CombatantRow from "./CombatantRow.svelte";
  import AddCombatants from "./AddCombatants.svelte";

  interface Props {
    /** The badge instance bound on mount — passed as a contribution prop so the same instance
     * the panel-tab chrome reads is the one this panel binds identity to. */
    badge: TurnBadge;
  }
  const { badge }: Props = $props();

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

  let busy = $state(false);
  let notation = $state("1d20");

  /** Runs `fn` under the panel's busy flag, surfacing a rejection through `ctx.notify`. Passed
   * down to the header/rows so every intent call shares one busy gate.
   * @param fn The async intent call to run. */
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
   * @returns A teardown removing the registration on unmount. */
  function trackRow(node: HTMLElement, id: string): { destroy: () => void } {
    rowEls.set(id, node);
    return { destroy: () => rowEls.delete(id) };
  }
  const reorder = createReorder(() => rows.map((r) => rowEls.get(r.doc.id)?.getBoundingClientRect() ?? new DOMRect()));

  function dispatchReorder(from: number, to: number): void {
    if (!selectedCombat || !can?.edit) return;
    const engine = selectedCombat.engine as { order: string[] };
    ctx.combat.reorder(selectedCombat.id, moveInOrder(engine.order, from, to));
  }

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

<section aria-label={ctx.t("combatTracker.title")}>
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
    <CombatHeader combat={selectedCombat} {rows} {busy} {run} bind:notation />
    <div class="rows">
      {#each rows as row, i (row.doc.id)}
        <div use:trackRow={row.doc.id} onkeydown={(e) => onRowKeydown(i, e)} role="presentation">
          <CombatantRow {row} combatId={selectedCombat.id} {registry} isTurn={(selectedCombat.engine as { turn: string | null }).turn === row.doc.id} {can} {busy} {run} {notation} {onDragStart} index={i} />
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
