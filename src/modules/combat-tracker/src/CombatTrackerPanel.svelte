<script lang="ts">
  import { onMount } from "svelte";
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import type { WireDocument } from "@shadowcat/core";
  import type { TurnBadge } from "./turnBadge";

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

  let busy = $state(false);

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

  {#if selectedCombat}
    <p data-testid="combat-tracker:selected">{selectedCombat.id}</p>
  {:else if ctx.role === "gm"}
    <p>{ctx.t("combatTracker.noCombat")}</p>
    <button type="button" data-testid="combat-tracker:create" disabled={busy} onclick={create}>{ctx.t("combatTracker.create")}</button>
  {:else}
    <p>{ctx.t("combatTracker.noCombatPlayer")}</p>
  {/if}
  {#if busy}<span aria-hidden="true">{ctx.t("combatTracker.busy")}</span>{/if}
</section>
