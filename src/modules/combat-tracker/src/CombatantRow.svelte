<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { resolveConditions, type CombatAffordances, type CombatantEngine, type Resource } from "@shadowcat/core";
  import { formatResource, firstChannel, type Row } from "./model";

  /** CombatantRow props. */
  interface Props {
    /** The row to render. */
    row: Row;
    /** The combat this row belongs to (for `combatId`-scoped intent calls). */
    combatId: string;
    /** Resource-registry entries, in registry order — one column per entry. */
    registry: [string, Resource][];
    /** Whether this row currently holds the turn. */
    isTurn: boolean;
    /** Advisory UI-gating affordances for the combat. */
    can: CombatAffordances;
    /** Disables every control while an intent is in flight. */
    busy: boolean;
    /** Runs an intent under the panel's shared busy/notify gate. */
    run: (fn: () => Promise<void>) => Promise<void>;
    /** The initiative-roll notation for this row's per-row roll. */
    notation: string;
    /** Starts a reorder drag from this row.
     * @param index This row's index within the visible order.
     * @param ev The originating pointer event. */
    onDragStart: (index: number, ev: PointerEvent) => void;
    /** This row's index within the visible order (passed to `onDragStart`). */
    index: number;
    /** Whether the panel is in the compact (narrow-viewport) layout. */
    compact?: boolean;
  }
  const { row, combatId, registry, isTurn, can, busy, run, notation, onDragStart, index, compact = false }: Props = $props();

  const ctx = getAppContext();
  // Reactive bridge: `ctx.documents` is a plain-callback store, not a Svelte rune (mirrors
  // CombatTrackerPanel's own `subscribeDocs`) — without it a condition applied/removed during
  // play never updates this row's glyphs.
  const subscribeDocs = createSubscriber((update) => ctx.documents.subscribe(update));

  const engine = $derived(row.doc.engine as CombatantEngine);
  const mayRoll = $derived(can.roll(row.doc.id));
  const mayEditResource = $derived(can.resource(row.doc.id));

  const conditions = $derived.by(() => {
    subscribeDocs();
    if (!row.art.tokenId) return [];
    const token = ctx.documents.get(row.art.tokenId);
    return token ? resolveConditions(token, ctx.documents) : [];
  });

  /**
   * Opens the row's own sheet: the token it names, else its linked actor. A no-op event row
   * (no token/actor) leaves the click without effect.
   * @example
   * ```
   * // private function; not part of the public API — invoked from the row's name button
   * openSheet();
   * ```
   */
  function openSheet(): void {
    if (row.art.tokenId) ctx.openDocument({ tokenId: row.art.tokenId });
    else if (row.art.actorId) ctx.openDocument({ docId: row.art.actorId });
  }

  /**
   * Writes this row's initiative from the input's text, clearing it (`null`) on a blank or
   * non-finite value.
   * @param value The initiative input's raw text.
   * @example
   * ```
   * // private function; not part of the public API — invoked from the initiative input
   * setInitiative("14");
   * ```
   */
  function setInitiative(value: string): void {
    const n = value === "" ? null : Number(value);
    ctx.combat.setInitiative(row.doc.id, Number.isFinite(n) ? n : null);
  }

  /**
   * Rolls this row's own initiative notation, posted to the first channel {@link firstChannel}
   * resolves. A no-op with a warning notice when no channel exists.
   * @example
   * ```
   * // private function; not part of the public API — invoked from the row's roll button
   * rollOne();
   * ```
   */
  function rollOne(): void {
    const channel = firstChannel(ctx.documents);
    if (!channel) {
      ctx.notify(ctx.t("combatTracker.noChannel"), "warning");
      return;
    }
    void run(() => ctx.combat.roll(combatId, channel, [{ combatant_id: row.doc.id, notation }]));
  }

  /**
   * Toggles this row's hidden state (`permissions.default` between `none` and readable).
   * @example
   * ```
   * // private function; not part of the public API — invoked from the row's hide/reveal button
   * toggleHidden();
   * ```
   */
  function toggleHidden(): void {
    ctx.combat.setHidden(row.doc.id, row.doc.permissions.default !== "none");
  }

  /**
   * Removes this row's combatant from the combat.
   * @example
   * ```
   * // private function; not part of the public API — invoked from the row's remove button
   * remove();
   * ```
   */
  function remove(): void {
    void run(async () => {
      ctx.combat.removeCombatant(combatId, row.doc.id);
    });
  }

  /**
   * Applies a relative change to one of this row's resources.
   * @param key The resource-registry key being adjusted.
   * @param amount The signed delta to apply.
   * @example
   * ```
   * // private function; not part of the public API — invoked from a resource's +/− buttons
   * resourceDelta("movement", -1);
   * ```
   */
  function resourceDelta(key: string, amount: number): void {
    void run(() => ctx.combat.modifyResource(combatId, row.doc.id, key, { kind: "delta", amount }));
  }

  /**
   * Sets one of this row's resources to an absolute value.
   * @param key The resource-registry key being set.
   * @param value The new absolute value.
   * @example
   * ```
   * // private function; not part of the public API — invoked from a resource's number input
   * resourceSet("movement", 2);
   * ```
   */
  function resourceSet(key: string, value: number): void {
    void run(() => ctx.combat.modifyResource(combatId, row.doc.id, key, { kind: "set", value }));
  }
</script>

<div class="row" class:turn={isTurn} class:compact aria-current={isTurn ? "true" : undefined} data-testid={"combat-tracker:row-" + row.doc.id}>
  {#if can.edit}
    <button type="button" class="drag-handle" aria-label={ctx.t("combatTracker.dragHandle")} onpointerdown={(e) => onDragStart(index, e)}>⠿</button>
  {/if}

  {#if row.kind === "event"}
    <span class="art">📣</span>
    <button type="button" class="name" onclick={openSheet}>{row.name ?? ctx.t("combatTracker.unnamed")}</button>
    <span class="lifespan">{engine.kind.type === "event" && engine.kind.lifespan !== null ? engine.kind.lifespan : "∞"}</span>
    {#if engine.kind.type === "event" && engine.kind.message}
      <span class="message">{engine.kind.message}</span>
    {/if}
  {:else}
    <button type="button" class="name" onclick={openSheet}>{row.name ?? ctx.t("combatTracker.unnamed")}</button>

    {#each conditions as c (c.id)}
      <span class="glyph" title={c.name}>{c.icon}</span>
    {/each}

    {#if can.edit || mayEditResource}
      <input type="number" aria-label="combatTracker.initiative" value={engine.initiative ?? ""}
        onchange={(e) => setInitiative((e.currentTarget as HTMLInputElement).value)} />
    {:else}
      <span>{engine.initiative ?? ""}</span>
    {/if}
    {#if engine.tiebreak}
      <sub>{engine.tiebreak}</sub>
    {/if}

    {#if mayRoll}
      <button type="button" data-testid={"combat-tracker:roll-" + row.doc.id} disabled={busy} onclick={rollOne}>{ctx.t("combatTracker.roll")}</button>
    {/if}

    {#if row.view === null}
      {#each registry as [key] (key)}
        <span class="resource" data-testid={"combat-tracker:resource-" + key}></span>
      {/each}
    {:else}
      {#each registry as [key, resource] (key)}
        {@const view = row.view.resources?.[key]}
        <span class="resource" data-testid={"combat-tracker:resource-" + key} title={view?.error ?? undefined}>
          {#if view?.error}
            ⚠
          {:else if resource.binding.kind === "tracked" && mayEditResource}
            <button type="button" disabled={busy} onclick={() => resourceDelta(key, -1)}>−</button>
            <input type="number" value={view?.current ?? ""} onchange={(e) => resourceSet(key, Number((e.currentTarget as HTMLInputElement).value))} />
            <button type="button" disabled={busy} onclick={() => resourceDelta(key, 1)}>+</button>
          {:else}
            {formatResource(view)}
          {/if}
        </span>
      {/each}
    {/if}
  {/if}

  {#if can.edit}
    <button type="button" data-testid={"combat-tracker:hide-" + row.doc.id} onclick={toggleHidden}>{row.doc.permissions.default === "none" ? ctx.t("combatTracker.visible") : ctx.t("combatTracker.hidden")}</button>
    <button type="button" disabled={isTurn} title={isTurn ? ctx.t("combatTracker.removeTurnHint") : undefined} onclick={remove}>{ctx.t("combatTracker.remove")}</button>
  {/if}
</div>

<style lang="scss">
  .row {
    display: grid;
    grid-auto-flow: column;
    align-items: center;
    gap: var(--space-1);

    .drag-handle,
    button,
    input {
      min-height: 32px;
    }
    .drag-handle {
      min-width: 44px;
    }

    &.turn {
      border-inline-start: 2px solid var(--accent);
    }

    &.compact {
      grid-auto-flow: row;
      grid-template-columns: 1fr;

      button,
      input {
        min-height: 44px;
      }
    }
  }
</style>
