<script lang="ts">
  import { escapeCoordinator, ESCAPE_PRIORITY } from "./escapeCoordinator";
  import { trapFocus } from "./focusTrap";

  let {
    title,
    message,
    confirmLabel,
    cancelLabel,
    danger = false,
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
    /** Styles the primary button as destructive and gives the dialog the alert-dialog role. */
    danger?: boolean;
    /** Called when the user confirms (primary button or Enter). */
    onConfirm: () => void;
    /** Called when the user dismisses (Cancel button or Escape). */
    onCancel: () => void;
  } = $props();

  const uid = $props.id();
  const titleId = `sc-dialog-title-${uid}`;
  const messageId = `sc-dialog-message-${uid}`;

  let dialogEl = $state<HTMLDivElement | null>(null);
  let confirmEl = $state<HTMLButtonElement | null>(null);

  // One Escape path: the shared coordinator at modal priority.
  $effect(() => escapeCoordinator.register(ESCAPE_PRIORITY.MODAL, onCancel));

  $effect(() => (dialogEl ? trapFocus(dialogEl) : undefined));

  $effect(() => {
    confirmEl?.focus();
  });

  // Enter confirms, except on a button (whose own activation handles Enter) or in a textarea.
  $effect(() => {
    const el = dialogEl;
    if (!el) return;
    const onKeydown = (event: KeyboardEvent): void => {
      if (event.key !== "Enter") return;
      const target = event.target;
      if (target instanceof HTMLButtonElement || target instanceof HTMLTextAreaElement) return;
      event.preventDefault();
      onConfirm();
    };
    el.addEventListener("keydown", onKeydown);
    return () => el.removeEventListener("keydown", onKeydown);
  });
</script>

<!-- The scrim is inert: only the two buttons or Escape dismiss, never an accidental outside click. -->
<div class="sc-dialog-scrim" role="presentation">
  <div
    class="sc-dialog"
    role={danger ? "alertdialog" : "dialog"}
    aria-modal="true"
    aria-labelledby={titleId}
    aria-describedby={messageId}
    tabindex="-1"
    data-testid="confirm-dialog"
    bind:this={dialogEl}
  >
    <h2 id={titleId}>{title}</h2>
    <p id={messageId}>{message}</p>
    <div class="actions">
      <button type="button" data-testid="confirm-dialog-cancel" onclick={onCancel}>{cancelLabel}</button>
      <button
        type="button"
        class="confirm"
        class:danger
        data-testid="confirm-dialog-confirm"
        bind:this={confirmEl}
        onclick={onConfirm}
      >{confirmLabel}</button>
    </div>
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
  button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
</style>
