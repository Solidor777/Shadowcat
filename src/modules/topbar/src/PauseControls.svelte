<script lang="ts">
  import { onMount } from "svelte";
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext, promptDialog } from "@shadowcat/ui-kit";
  import {
    buildUpdate,
    resolveSessionPause,
    type KeybindingOptions,
    type KeybindingRegistration,
    type SessionState,
    type WireDocument,
    type WorldSettingsEngine,
  } from "@shadowcat/core";

  /** PauseControls props. */
  interface Props {
    /** Registers the pause keybinding with the module's keybinding controller. Absent where no
     * controller exists (a bare render); the buttons still work. */
    registerKeybinding?: (bindingId: string, opts: KeybindingOptions) => KeybindingRegistration;
  }
  const { registerKeybinding }: Props = $props();

  const ctx = getAppContext();
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));

  /** Longest pause message, in characters; the server refuses a longer one. */
  const MAX_MESSAGE_CHARS = 280;
  /** The default key combo of the pause toggle. */
  const PAUSE_COMBO = "Alt+KeyP";
  /** The keybinding id the toggle registers under. */
  const PAUSE_BINDING = "toggle-pause";

  const isGm = $derived(ctx.role === "gm");

  const settingsDoc = $derived.by((): WireDocument | undefined => {
    subscribe();
    return ctx.documents.query("world-settings")[0];
  });
  const pause = $derived.by(() => {
    subscribe();
    return resolveSessionPause(ctx.documents);
  });

  /**
   * Writes the world-settings `session` block. The OCC pre-image is the stored block exactly
   * as it is (`null` when absent), never a defaulted copy. A silent no-op while the
   * world-settings document is absent: it is seeded on the GM's first Welcome, so that is only a
   * startup race.
   * @param next The whole new session block.
   * @example
   * ```
   * // private function; not part of the public API — invoked by the pause toggle and audio switch
   * writeSession({ paused: true, pauseAudio: false });
   * ```
   */
  function writeSession(next: SessionState): void {
    const doc = settingsDoc;
    if (!doc) return;
    const old = (doc.engine as WorldSettingsEngine).session ?? null;
    ctx.dispatchIntent([buildUpdate(doc.id, [{ path: "/engine/session", old, value: next }])]);
  }

  /**
   * Validates a pause message the way the server will: at most {@link MAX_MESSAGE_CHARS}
   * characters and no control character.
   * @param value The text typed into the prompt.
   * @returns A translated error message, or `null` when the text is acceptable.
   * @example
   * ```
   * // private function; not part of the public API — passed to promptDialog as its validator
   * validateMessage("Back in five");
   * ```
   */
  function validateMessage(value: string): string | null {
    if (Array.from(value).length > MAX_MESSAGE_CHARS) return ctx.t("topbar.pause.messageTooLong", { max: MAX_MESSAGE_CHARS });
    if (/\p{Cc}/u.test(value)) return ctx.t("topbar.pause.messageInvalid");
    return null;
  }

  /**
   * Resumes a paused session, or asks the GM for an optional message and pauses it. Dismissing
   * the prompt leaves the session running. The pause-audio choice carries over unchanged.
   * @returns Resolves once the intent is dispatched or the prompt is dismissed.
   * @example
   * ```
   * // private function; not part of the public API — invoked from the toggle button and keybinding
   * await togglePause();
   * ```
   */
  async function togglePause(): Promise<void> {
    if (!isGm) return;
    const { paused, pauseAudio } = pause;
    if (paused) {
      writeSession({ paused: false, pauseAudio });
      return;
    }
    const text = await promptDialog(ctx, {
      titleKey: "topbar.pause.promptTitle",
      messageKey: "topbar.pause.promptMessage",
      placeholder: ctx.t("topbar.pause.promptPlaceholder"),
      confirmLabelKey: "topbar.pause.promptConfirm",
      cancelLabelKey: "topbar.pause.promptCancel",
      validate: validateMessage,
    });
    if (text === null) return;
    const message = text.trim();
    writeSession({ paused: true, pauseAudio: pause.pauseAudio, ...(message === "" ? {} : { message }) });
  }

  /**
   * Flips whether the pause also pauses the world's audio, keeping the pause state and message.
   * @param on The checkbox's new state.
   * @example
   * ```
   * // private function; not part of the public API — invoked from the audio checkbox
   * setPauseAudio(true);
   * ```
   */
  function setPauseAudio(on: boolean): void {
    writeSession({ paused: pause.paused, pauseAudio: on, ...(pause.message === null ? {} : { message: pause.message }) });
  }

  onMount(() => {
    if (!isGm || !registerKeybinding) return;
    const reg = registerKeybinding(PAUSE_BINDING, {
      defaultCombo: PAUSE_COMBO,
      labelKey: "topbar.pause.binding",
      action: () => void togglePause(),
    });
    return reg.dispose;
  });
</script>

{#if isGm}
  <div class="pause-controls">
    <button
      type="button"
      class="pause-toggle"
      data-testid="topbar-pause-toggle"
      aria-pressed={pause.paused}
      onclick={() => void togglePause()}
    >
      {pause.paused ? ctx.t("topbar.pause.resume") : ctx.t("topbar.pause.pause")}
    </button>
    <label class="pause-audio">
      <input
        type="checkbox"
        data-testid="topbar-pause-audio"
        checked={pause.pauseAudio}
        onchange={(e) => setPauseAudio((e.currentTarget as HTMLInputElement).checked)}
      />
      {ctx.t("topbar.pause.audio")}
    </label>
  </div>
{/if}

<style lang="scss">
  .pause-controls {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .pause-toggle {
    min-width: 44px;
    min-height: 44px; /* touch target */
    padding: 0 var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    background: var(--surface-raised);
    color: var(--text-primary);
    cursor: pointer;
    &[aria-pressed="true"] {
      border-color: var(--accent);
    }
    &:focus-visible {
      outline: 2px solid var(--accent);
      outline-offset: 2px;
    }
  }
  .pause-audio {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    min-height: 44px; /* touch target */
    white-space: nowrap;
  }
</style>
