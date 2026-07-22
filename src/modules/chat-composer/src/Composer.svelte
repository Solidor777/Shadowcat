<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { actorDisplayName, MAX_MESSAGE_CHARS, type WireActorOwnerRef, type WireAudience, type WireDocument } from "@shadowcat/core";

  let { channel, audience, placeholderName }: { channel: string; audience: WireAudience; placeholderName: string } = $props();

  const ctx = getAppContext();
  const t = ctx.t;

  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));

  // "Speak as" options: the default (no attribution) plus every actor doc the current
  // user may speak as — own actors only for a Player, ALL actors for a GM (spec §8).
  // Reactive to actor creation/ownership changes via the store subscriber bridge.
  const speakableActors = $derived.by((): WireDocument[] => {
    subscribe();
    const all = ctx.documents.query("actor");
    return ctx.role === "gm" ? all : all.filter((doc) => doc.owner === ctx.selfId);
  });

  // Sticky per session (component-local `$state`, not persisted) — spec §8. Empty string
  // is the sentinel for "Myself" (the default, no-attribution option).
  let selectedActorId = $state("");

  // Prunes a dangling selection: if the selected actor leaves `speakableActors`
  // (deleted, or ownership transferred away from the current user), the select
  // would otherwise render blank (selectedIndex -1) while sends kept attaching
  // the now-nonexistent actor_id. Resets to "" (Myself) whenever the current
  // selection no longer resolves to a live, speakable actor doc.
  $effect(() => {
    if (selectedActorId && !speakableActors.some((a) => a.id === selectedActorId)) {
      selectedActorId = "";
    }
  });

  // Counter shows only when the author is nearing the server cap (MAX_MESSAGE_CHARS,
  // chat/mod.rs) — not on every keystroke, to avoid a permanently-visible chrome element.
  const COUNTER_THRESHOLD = MAX_MESSAGE_CHARS - 200;

  let value = $state("");
  let textarea = $state<HTMLTextAreaElement | undefined>(undefined);
  // The server's player-presentable rejection reason for the last send, shown
  // inline so a refused message does not vanish silently. Already classified
  // server-side (authorization/existence/internal errors are generic there).
  let errorMsg = $state<string | null>(null);

  const trimmed = $derived(value.trim());
  // Cap/counter/send-gating derive from the TRIMMED length, matching what send()
  // actually transmits and what the server validates (chat/mod.rs MAX_MESSAGE_CHARS).
  // Known, fail-safe divergence: JS .length counts UTF-16 code units while the server
  // counts Unicode scalar values (chars().count()) — the client can only over-block
  // near the cap, never under-block, so this asymmetry is safe.
  const overLimit = $derived(trimmed.length > MAX_MESSAGE_CHARS);
  const showCounter = $derived(trimmed.length > COUNTER_THRESHOLD);
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
    errorMsg = null;
    // /-commands (e.g. "/roll 1d6") ride verbatim — the server (chat::parse_command)
    // is the sole parser; the composer never inspects or branches on content shape.
    const actorOwner: WireActorOwnerRef | undefined = selectedActorId ? { kind: "actor", actor_id: selectedActorId } : undefined;
    // Clear the input optimistically; a server rejection surfaces inline via `errorMsg`
    // (correlated by request_id under the seam) instead of the message vanishing.
    Promise.resolve(
      ctx.chat.send(actorOwner ? { channel, content: trimmed, audience, actorOwner } : { channel, content: trimmed, audience }),
    ).catch((e: unknown) => {
      errorMsg = e instanceof Error ? e.message : t("chat.composer.sendFailed");
    });
    value = "";
    queueMicrotask(autoGrow);
  }

  // Clear a stale rejection notice as soon as the author starts a fresh message.
  function onInput(): void {
    errorMsg = null;
    autoGrow();
  }

  function onKeydown(e: KeyboardEvent): void {
    // Ignore Enter while an IME composition is in progress (CJK/Japanese/Korean input) —
    // the Enter here commits the composed candidate, it must not also send the message.
    if (e.isComposing) return;
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      send();
    }
    // Shift+Enter falls through to the textarea's default newline insertion.
  }
</script>

<div class="composer">
  <label class="visually-hidden" for="chat-composer-speak-as">{t("chat.composer.speakAs")}</label>
  <select id="chat-composer-speak-as" bind:value={selectedActorId}>
    <option value="">{t("chat.composer.myself")}</option>
    {#each speakableActors as actor (actor.id)}
      <option value={actor.id}>{actorDisplayName({ name: actor.name, displayName: (actor.engine as { displayName?: string } | undefined)?.displayName })}</option>
    {/each}
  </select>
  <label class="visually-hidden" for="chat-composer-input">{placeholder}</label>
  <textarea
    id="chat-composer-input"
    bind:this={textarea}
    bind:value
    {placeholder}
    onkeydown={onKeydown}
    oninput={onInput}
    rows="1"
  ></textarea>
  <button type="button" onclick={send} disabled={!canSend}>{t("chat.composer.send")}</button>
</div>
{#if errorMsg}
  <div class="send-error" role="alert">{errorMsg}</div>
{/if}
{#if showCounter}
  <div class="counter" class:over={overLimit}>{t("chat.composer.count", { used: trimmed.length, max: MAX_MESSAGE_CHARS })}</div>
{/if}

<style lang="scss">
  .composer {
    display: flex;
    align-items: flex-end;
    gap: var(--space-1);
  }
  select {
    flex: 0 0 auto;
    min-height: 44px;
    max-width: 8em;
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
      color: var(--danger);
    }
  }
  .send-error {
    margin-top: var(--space-1);
    font-size: 0.85em;
    color: var(--danger);
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
