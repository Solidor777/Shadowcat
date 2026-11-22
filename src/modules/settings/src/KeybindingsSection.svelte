<script lang="ts">
  import { onDestroy } from "svelte";
  import { createSubscriber } from "svelte/reactivity";
  import { describeCombo, isModifierOnly, normalizeCombo, type ModuleContext } from "@shadowcat/core";
  import { ESCAPE_PRIORITY, escapeCoordinator, getAppContext } from "@shadowcat/ui-kit";

  let {
    keybindings,
  }: {
    /** The settings module's `ctx.keybindings`; lists every module's bindings and rebinds them. */
    keybindings: ModuleContext["keybindings"];
  } = $props();

  const { t } = getAppContext();

  // Bridges the controller's subscribe/snapshot to Svelte's reactivity, the same pattern
  // `Settings.svelte` uses for the contribution registry.
  const subscribeBindings = createSubscriber((update) => keybindings.subscribe(update));
  const bindings = $derived.by(() => {
    subscribeBindings();
    return keybindings.allBindings();
  });

  /** The binding whose next keystroke is being captured, or `null` when no capture is armed. */
  let capturing = $state<{
    /** The binding's module id. */
    moduleId: string;
    /** The binding's id within the module. */
    bindingId: string;
  } | null>(null);
  /** Removes the armed capture's window listener and Escape registration. */
  let stopCapture: (() => void) | null = null;

  /**
   * Ends an armed capture without changing any binding.
   * @example
   * ```
   * // private function; not part of the public API — wired to Escape and the Cancel button
   * cancelCapture();
   * ```
   */
  function cancelCapture(): void {
    stopCapture?.();
    stopCapture = null;
    capturing = null;
  }

  /**
   * Arms capture for one binding: the next non-modifier keystroke becomes its combo. A capture-phase
   * window listener consumes the keystroke so no other binding or handler also fires on it, and an
   * Escape registration (and the Cancel button) always leaves capture. A combo the browser or OS
   * reserves never reaches the page, so capture simply keeps waiting; Escape is the way out.
   * @param moduleId The binding's module id.
   * @param bindingId The binding's id within the module.
   * @example
   * ```
   * // private function; not part of the public API — wired to each row's change button
   * startCapture("example", "ping");
   * ```
   */
  function startCapture(moduleId: string, bindingId: string): void {
    cancelCapture();
    capturing = { moduleId, bindingId };
    /**
     * Records the first non-modifier keystroke as the armed binding's combo.
     * @param e The captured keydown.
     * @example
     * ```
     * // private closure; not part of the public API — attached to `window` while capture is armed
     * window.addEventListener("keydown", onKeydown, true);
     * ```
     */
    const onKeydown = (e: KeyboardEvent): void => {
      // Escape cancels through the coordinator; a lone modifier waits for its main key.
      if (e.code === "Escape" || isModifierOnly(e)) return;
      e.preventDefault();
      e.stopPropagation();
      keybindings.rebind(moduleId, bindingId, normalizeCombo(e));
      cancelCapture();
    };
    window.addEventListener("keydown", onKeydown, true);
    const disposeEscape = escapeCoordinator.register(ESCAPE_PRIORITY.OVERLAY, cancelCapture);
    stopCapture = () => {
      window.removeEventListener("keydown", onKeydown, true);
      disposeEscape();
    };
  }

  onDestroy(cancelCapture);
</script>

<div class="keybindings">
  {#each bindings as binding (`${binding.moduleId}:${binding.bindingId}`)}
    {@const name = t(binding.labelKey)}
    <div class="row">
      <span class="name">{name}</span>
      <span class="module">{binding.moduleId}</span>
      <span class="combo">
        {#if capturing?.moduleId === binding.moduleId && capturing.bindingId === binding.bindingId}
          {t("settings.keybindings.prompt")}
        {:else if binding.combo === null}
          {t("settings.keybindings.unbound")}
        {:else}
          {describeCombo(binding.combo, t)}
        {/if}
      </span>
      {#if capturing?.moduleId === binding.moduleId && capturing.bindingId === binding.bindingId}
        <button type="button" onclick={cancelCapture}>{t("settings.keybindings.cancel")}</button>
      {:else}
        <button
          type="button"
          aria-label={t("settings.keybindings.rebind", { name })}
          onclick={() => startCapture(binding.moduleId, binding.bindingId)}
        >
          {t("settings.keybindings.rebindButton")}
        </button>
        <button
          type="button"
          aria-label={t("settings.keybindings.clear", { name })}
          disabled={binding.combo === null}
          onclick={() => keybindings.rebind(binding.moduleId, binding.bindingId, null)}
        >
          {t("settings.keybindings.clearButton")}
        </button>
      {/if}
    </div>
  {:else}
    <p>{t("settings.keybindings.empty")}</p>
  {/each}
</div>

<style lang="scss">
  .keybindings {
    display: grid;
    gap: var(--space-2);
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }
  .name {
    flex: 1 1 8rem;
  }
  .module {
    color: var(--text-muted);
    font-size: 0.875rem;
  }
  @media (pointer: coarse) {
    button {
      min-height: var(--input-height-coarse);
    }
  }
</style>
