<script lang="ts">
  import { untrack } from "svelte";
  import { escapeCoordinator, ESCAPE_PRIORITY } from "./escapeCoordinator";
  import { trapFocus } from "./focusTrap";

  let {
    title,
    message,
    confirmLabel,
    cancelLabel,
    danger = false,
    initialValue = "",
    placeholder = "",
    validate,
    onConfirm,
    onCancel,
  }: {
    /** The heading. */
    title: string;
    /** The body text. */
    message: string;
    /** The primary button's label. */
    confirmLabel: string;
    /** The cancel button's label. */
    cancelLabel: string;
    /** Styles the primary button as destructive. */
    danger?: boolean;
    /** The text field's starting value; selected on open. */
    initialValue?: string;
    /** The text field's placeholder. */
    placeholder?: string;
    /** Returns an error message for an invalid value, or `null` when valid. */
    validate?: (value: string) => string | null;
    /** Called with the entered text when the user submits (primary button or Enter). */
    onConfirm: (value: string) => void;
    /** Called when the user dismisses (Cancel button or Escape). */
    onCancel: () => void;
  } = $props();

  const uid = $props.id();
  const titleId = `sc-dialog-title-${uid}`;
  const messageId = `sc-dialog-message-${uid}`;
  const errorId = `sc-dialog-error-${uid}`;

  // The prop seeds the field once; later edits belong to the user, so the read is untracked.
  let value = $state(untrack(() => initialValue));
  let dialogEl = $state<HTMLDivElement | null>(null);
  let inputEl = $state<HTMLInputElement | null>(null);

  const error = $derived(validate ? validate(value) : null);

  /**
   * Submits the entered text unless validation currently fails.
   * @example
   * submit();
   */
  function submit(): void {
    if (error !== null) return;
    onConfirm(value);
  }

  // One Escape path: the shared coordinator at modal priority.
  $effect(() => escapeCoordinator.register(ESCAPE_PRIORITY.MODAL, onCancel));

  $effect(() => (dialogEl ? trapFocus(dialogEl) : undefined));

  // Enter submits from the text field; a button keeps its own Enter activation. Handled here
  // (and the native implicit form submit suppressed) so a failed validation blocks it
  // identically on every browser.
  $effect(() => {
    const el = dialogEl;
    if (!el) return;
    const onKeydown = (event: KeyboardEvent): void => {
      if (event.key !== "Enter" || event.target instanceof HTMLButtonElement) return;
      event.preventDefault();
      submit();
    };
    el.addEventListener("keydown", onKeydown);
    return () => el.removeEventListener("keydown", onKeydown);
  });

  // Focus the field with its text selected, so retyping needs no select-all.
  $effect(() => {
    inputEl?.focus();
    inputEl?.select();
  });
</script>

<!-- The scrim is inert: only the two buttons or Escape dismiss, never an accidental outside click. -->
<div class="sc-dialog-scrim" role="presentation">
  <div
    class="sc-dialog"
    role="dialog"
    aria-modal="true"
    aria-labelledby={titleId}
    aria-describedby={messageId}
    tabindex="-1"
    data-testid="prompt-dialog"
    bind:this={dialogEl}
  >
    <h2 id={titleId}>{title}</h2>
    <p id={messageId}>{message}</p>
    <form
      onsubmit={(e) => {
        e.preventDefault();
        submit();
      }}
    >
      <input
        type="text"
        data-testid="prompt-dialog-input"
        {placeholder}
        aria-invalid={error !== null}
        aria-describedby={error !== null ? errorId : undefined}
        bind:value
        bind:this={inputEl}
      />
      {#if error !== null}
        <p class="error" id={errorId} data-testid="prompt-dialog-error">{error}</p>
      {/if}
      <div class="actions">
        <button type="button" data-testid="prompt-dialog-cancel" onclick={onCancel}>{cancelLabel}</button>
        <button
          type="submit"
          class="confirm"
          class:danger
          data-testid="prompt-dialog-confirm"
          disabled={error !== null}
        >{confirmLabel}</button>
      </div>
    </form>
  </div>
</div>

<style lang="scss">
  .sc-dialog-scrim {
    position: fixed;
    inset: 0;
    background: var(--scrim);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: var(--z-popover);
  }
  .sc-dialog {
    background: var(--surface-raised);
    color: var(--text-primary);
    border: 1px solid var(--border);
    border-radius: var(--radius-2);
    padding: var(--space-3);
    max-width: min(90vw, 28rem);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  h2 {
    margin: 0;
    font-size: var(--font-size-md);
  }
  p {
    margin: 0;
  }
  form {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  input {
    min-height: 44px;
  }
  .error {
    color: var(--danger);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }
  /* Touch-target floor (mobile invariant). */
  button {
    min-height: 44px;
    padding: 0 var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    background: var(--surface-raised);
    color: inherit;
  }
  .confirm {
    background: var(--accent);
    color: var(--on-accent);
  }
  .confirm.danger {
    background: var(--danger);
    color: var(--on-accent);
  }
  button:focus-visible,
  input:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
</style>
