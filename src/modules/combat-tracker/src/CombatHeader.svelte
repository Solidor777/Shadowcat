<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import type { WireDocument, CombatEngine } from "@shadowcat/core";
  import { rollTargets, firstChannel, type Row } from "./model";

  interface Props {
    /** The combat document this header controls. */
    combat: WireDocument;
    /** The panel's current rows, for "Roll all"'s target set. */
    rows: Row[];
    /** Disables every control while an intent is in flight. */
    busy: boolean;
    /** Runs an intent under the panel's shared busy/notify gate. */
    run: (fn: () => Promise<void>) => Promise<void>;
    /** The initiative-roll notation, bindable so the panel can persist it across renders. */
    notation: string;
    /** Whether the panel is in the compact (narrow-viewport) layout. */
    compact?: boolean;
  }
  let { combat, rows, busy, run, notation = $bindable(), compact = false }: Props = $props();

  const ctx = getAppContext();

  const engine = $derived(combat.engine as CombatEngine);
  const can = $derived(ctx.combat.canAct(combat.id));

  /** Two-click confirm window for `End`: the first click arms `confirming`; a second click
   * within {@link CONFIRM_WINDOW_MS} runs the end; the arm expires otherwise. A native
   * `window.confirm` is neither testable nor touch-friendly. */
  const CONFIRM_WINDOW_MS = 5000;
  let confirming = $state(false);
  let confirmTimer: ReturnType<typeof setTimeout> | undefined;

  function clickEnd(): void {
    if (!confirming) {
      confirming = true;
      confirmTimer = setTimeout(() => (confirming = false), CONFIRM_WINDOW_MS);
      return;
    }
    clearTimeout(confirmTimer);
    confirming = false;
    void run(() => ctx.combat.end(combat.id));
  }

  function rollAll(): void {
    const channel = firstChannel(ctx.documents);
    if (!channel) {
      ctx.notify(ctx.t("combatTracker.noChannel"), "warning");
      return;
    }
    const targets = rollTargets(rows, ctx.role === "gm" ? "gm" : "player", ctx.selfId);
    if (targets.length === 0) return;
    void run(() =>
      ctx.combat.roll(
        combat.id,
        channel,
        targets.map((combatant_id) => ({ combatant_id, notation })),
      ),
    );
  }
</script>

<header class:compact>
  <span>{engine.round > 0 ? ctx.t("combatTracker.round", { n: engine.round }) : ctx.t("combatTracker.notStarted")}</span>
  <span>{engine.turn ? ctx.t("combatTracker.turn") : ctx.t("combatTracker.noTurn")}</span>

  {#if can.start}
    <button type="button" disabled={busy} onclick={() => void run(() => ctx.combat.start(combat.id))}>{ctx.t("combatTracker.start")}</button>
  {/if}
  {#if can.pause}
    <button type="button" disabled={busy} onclick={() => void run(() => ctx.combat.pause(combat.id))}>{ctx.t("combatTracker.pause")}</button>
  {/if}
  {#if can.advance}
    {#if ctx.role === "gm"}
      <button type="button" disabled={busy} onclick={() => void run(() => ctx.combat.advance(combat.id))}>{ctx.t("combatTracker.advance")}</button>
    {:else}
      <button type="button" data-testid="combat-tracker:end-my-turn" disabled={busy} onclick={() => void run(() => ctx.combat.advance(combat.id))}>{ctx.t("combatTracker.endMyTurn")}</button>
    {/if}
  {/if}
  {#if can.rewind}
    <button type="button" data-testid="combat-tracker:rewind" disabled={busy} onclick={() => void run(() => ctx.combat.rewind(combat.id))}>{ctx.t("combatTracker.rewind")}</button>
  {/if}
  {#if can.sort}
    <button type="button" disabled={busy} onclick={() => void run(() => ctx.combat.sort(combat.id))}>{ctx.t("combatTracker.sort")}</button>
  {/if}
  {#if can.end}
    <button type="button" data-testid="combat-tracker:end" disabled={busy} onclick={clickEnd}>{confirming ? ctx.t("combatTracker.endConfirm") : ctx.t("combatTracker.end")}</button>
  {/if}

  <label>
    {ctx.t("combatTracker.notation")}
    <input type="text" aria-label="combatTracker.notation" bind:value={notation} />
  </label>
  <button type="button" data-testid="combat-tracker:roll-all" disabled={busy} onclick={rollAll}>{ctx.t("combatTracker.rollAll")}</button>

  {#if ctx.role === "gm"}
    <button type="button" onclick={() => ctx.panels.open("game-settings:panel")}>{ctx.t("combatTracker.settings")}</button>
    {#if !engine.active}
      <button type="button" disabled={busy} onclick={() => void run(async () => { ctx.combat.deleteCombat(combat.id); })}>{ctx.t("combatTracker.delete")}</button>
    {/if}
  {/if}
</header>

<style lang="scss">
  header {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-1);

    button,
    input {
      min-height: 32px;
    }

    &.compact {
      button,
      input {
        min-height: 44px;
      }
    }
  }
</style>
