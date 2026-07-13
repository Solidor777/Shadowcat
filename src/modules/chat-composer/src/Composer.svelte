<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import { MAX_MESSAGE_CHARS, type WireAudience } from "@shadowcat/core";

  let { channel, audience, placeholderName }: { channel: string; audience: WireAudience; placeholderName: string } = $props();

  const ctx = getAppContext();
  const t = ctx.t;

  // Counter shows only when the author is nearing the server cap (MAX_MESSAGE_CHARS,
  // chat/mod.rs) — not on every keystroke, to avoid a permanently-visible chrome element.
  const COUNTER_THRESHOLD = MAX_MESSAGE_CHARS - 200;

  let value = $state("");
  let textarea = $state<HTMLTextAreaElement | undefined>(undefined);

  const trimmed = $derived(value.trim());
  const overLimit = $derived(value.length > MAX_MESSAGE_CHARS);
  const showCounter = $derived(value.length > COUNTER_THRESHOLD);
  const canSend = $derived(trimmed.length > 0 && !overLimit);

  const placeholder = $derived(audience.kind === "gm_only" ? t("chat.composer.placeholderGm") : t("chat.composer.placeholder", { name: placeholderName }));

  // Auto-grow: reset height before measuring scrollHeight, or the textarea can
  // never shrink back down after a multi-line message is cleared.
  function autoGrow(): void {
    if (!textarea) return;
    textarea.style.height = "auto";
    textarea.style.height = `${textarea.scrollHeight}px`;
  }

  function send(): void {
    if (!canSend) return;
    // /-commands (e.g. "/roll 1d6") ride verbatim — the server (chat::parse_command)
    // is the sole parser; the composer never inspects or branches on content shape.
    ctx.chat.send({ channel, content: trimmed, audience });
    value = "";
    queueMicrotask(autoGrow);
  }

  function onKeydown(e: KeyboardEvent): void {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      send();
    }
    // Shift+Enter falls through to the textarea's default newline insertion.
  }
</script>

<div class="composer">
  <label class="visually-hidden" for="chat-composer-input">{placeholder}</label>
  <textarea
    id="chat-composer-input"
    bind:this={textarea}
    bind:value
    {placeholder}
    onkeydown={onKeydown}
    oninput={autoGrow}
    rows="1"
  ></textarea>
  <button type="button" onclick={send} disabled={!canSend}>{t("chat.composer.send")}</button>
</div>
{#if showCounter}
  <div class="counter" class:over={overLimit}>{t("chat.composer.count", { used: value.length, max: MAX_MESSAGE_CHARS })}</div>
{/if}

<style lang="scss">
  .composer {
    display: flex;
    align-items: flex-end;
    gap: var(--space-1);
  }
  textarea {
    flex: 1 1 auto;
    resize: none;
    max-height: 12em;
    min-height: 44px;
    padding: var(--space-1);
  }
  button {
    flex: 0 0 auto;
    min-height: 44px;
    min-width: 44px;
  }
  .counter {
    text-align: right;
    font-size: 0.85em;
    &.over {
      color: var(--color-danger, #c0392b);
    }
  }
  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
  }
</style>
