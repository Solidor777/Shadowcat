<script lang="ts">
  // A portal trigger's editor. "Single destination" edits a `teleport` effect (one fixed
  // `PortalTarget`); "Several destinations" edits a `portal_choice` effect (a prompt plus a list of
  // labelled options, each with its own target and a "show scene name" flag). Switching the mode
  // replaces the effect, carrying the first destination across. `effect` is `$bindable`: it lives
  // inside the `$state`-proxied `ToolController.regionTriggers` tree owned by `ToolRail.svelte`,
  // and a plain (non-bindable) prop mutated from a CHILD component is a dev-mode ownership
  // violation in Svelte 5 (`ownership_invalid_mutation`) — `bind:effect` at the call site is the
  // sanctioned two-way seam across that component boundary.
  import { getAppContext } from "@shadowcat/ui-kit";
  import type { TriggerEffect } from "@shadowcat/core";
  import type { ToolController } from "./controller.svelte";
  import PortalTargetFields from "./PortalTargetFields.svelte";

  let { effect = $bindable(), row, controller }: {
    /** The trigger's effect, a `teleport` or a `portal_choice`; replaced when the mode switches. */
    effect: TriggerEffect;
    /** This row's index into `controller.regionTriggers`, for pick-on-stage targeting. */
    row: number;
    /** The shared tool-controller instance (pick-on-stage state + trigger). */
    controller: ToolController;
  } = $props();

  const ctx = getAppContext();
  const { t } = ctx;

  /** Switch between one fixed destination and several. The first destination (the single
   * target, or the first option's target) survives the switch; everything else starts fresh.
   * @param several `true` for the `portal_choice` mode, `false` for the `teleport` mode.
   * @example
   * ```
   * declare const editor: { setSeveral(several: boolean): void };
   * editor.setSeveral(true);
   * ```
   */
  function setSeveral(several: boolean): void {
    if (several === (effect.type === "portal_choice")) return;
    if (effect.type === "teleport") {
      effect = { type: "portal_choice", prompt: null, options: [{ label: "", target: effect.target, reveal_scene: false }] };
    } else if (effect.type === "portal_choice") {
      const first = effect.options[0]?.target ?? { scene: null, x: 0, y: 0, elevation: null, vfx: null };
      effect = { type: "teleport", target: first };
    }
  }

  /** Append a blank option (empty label, no scene, origin, scene name hidden).
   * @example
   * ```
   * declare const editor: { addOption(): void };
   * editor.addOption();
   * ```
   */
  function addOption(): void {
    if (effect.type !== "portal_choice") return;
    effect.options.push({ label: "", target: { scene: null, x: 0, y: 0, elevation: null, vfx: null }, reveal_scene: false });
  }
</script>

<div class="portal-editor" data-testid="region-trigger-portal-editor">
  <label class="mode">
    <input
      type="checkbox"
      data-testid="region-trigger-portal-several"
      checked={effect.type === "portal_choice"}
      onchange={(e) => setSeveral(e.currentTarget.checked)}
    />
    <span>{t("tools.triggerPortalSeveral")}</span>
  </label>
  {#if effect.type === "teleport"}
    <PortalTargetFields
      bind:target={effect.target}
      picking={controller.pickingPortalRow === row}
      onPick={() => controller.beginPickPortalTarget(row)}
    />
  {:else if effect.type === "portal_choice"}
    <input
      data-testid="region-trigger-portal-prompt"
      aria-label={t("tools.triggerPortalPrompt")}
      placeholder={t("tools.triggerPortalPrompt")}
      value={effect.prompt ?? ""}
      onchange={(e) => {
        if (effect.type === "portal_choice") effect.prompt = e.currentTarget.value === "" ? null : e.currentTarget.value;
      }}
    />
    {#each effect.options as option, i (option)}
      <div class="option" data-testid="region-trigger-portal-option">
        <input
          data-testid="region-trigger-portal-label"
          aria-label={t("tools.triggerPortalLabel")}
          placeholder={t("tools.triggerPortalLabel")}
          bind:value={option.label}
        />
        <label class="reveal">
          <input type="checkbox" data-testid="region-trigger-portal-reveal" bind:checked={option.reveal_scene} />
          <span>{t("tools.triggerPortalReveal")}</span>
        </label>
        <button
          type="button"
          data-testid="region-trigger-portal-remove"
          title={t("tools.triggerPortalRemoveOption")}
          aria-label={t("tools.triggerPortalRemoveOption")}
          onclick={() => {
            if (effect.type === "portal_choice") effect.options.splice(i, 1);
          }}
        >×</button>
        <PortalTargetFields
          bind:target={option.target}
          picking={controller.pickingPortalRow === row && controller.pickingPortalOption === i}
          onPick={() => controller.beginPickPortalTarget(row, i)}
        />
      </div>
    {/each}
    <button type="button" data-testid="region-trigger-portal-add" onclick={addOption}>{t("tools.triggerPortalAddOption")}</button>
  {/if}
</div>

<style lang="scss">
  .portal-editor {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  button {
    min-height: var(--input-height-coarse);
    min-width: var(--input-height-coarse);
    box-sizing: border-box;
  }
  .mode,
  .reveal {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
  }
  .option {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-1);
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
  }
</style>
