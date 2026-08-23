<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { actorDisplayName, resolveTokenActor, MAX_MESSAGE_CHARS, type WireActorOwnerRef, type WireAudience, type WireDocument, type WireSearchHit, type SubscriptionHandle } from "@shadowcat/core";

  let {
    channel,
    audience,
    placeholderName,
  }: {
    /** The client-chosen channel label to post under — a display grouping with zero server-enforced meaning. */
    channel: string;
    /** The intended readership to send with; the server, not `channel`, is what actually restricts readers. */
    audience: WireAudience;
    /** The active view's display name, shown in the composer's placeholder text (never the sender's own name). */
    placeholderName: string;
  } = $props();

  const ctx = getAppContext();
  const t = ctx.t;

  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));

  // "Speak as" options: the default (no attribution) plus every actor doc the current user
  // may OFFER to speak as — own actors for a Player, all actors for a GM. This list is a
  // client affordance only: the server independently re-authorizes the actual choice at send
  // in `handle_send_message` (a Player must own the named actor; a GM may name
  // any actor in the world), so an offer this list gets wrong can only be rejected, never
  // silently accepted. Reactive to actor creation/ownership changes via the store subscriber
  // bridge.
  const speakableActors = $derived.by((): WireDocument[] => {
    subscribe();
    const all = ctx.documents.query("actor");
    return ctx.role === "gm" ? all : all.filter((doc) => doc.owner === ctx.selfId);
  });

  // Sticky per session (component-local `$state`, not persisted). Empty string is the
  // sentinel for "Myself" (the default, no-attribution option).
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

  // Counter shows only when the author is nearing the server cap
  // (`MAX_MESSAGE_CHARS`) — not on every keystroke, to avoid a
  // permanently-visible chrome element.
  const COUNTER_THRESHOLD = MAX_MESSAGE_CHARS - 200;

  let value = $state("");
  let textarea = $state<HTMLTextAreaElement | undefined>(undefined);

  // `@doc` trigger: a searchable document/token picker parallel in UI weight to the "Speak
  // as" picker above, wired to the existing `searchDocuments` AppContext seam — no new
  // search/lookup code. Live-search subscription mirrors ActorsPanel's own pattern
  // (torn down/recreated on every query change, guarded against a stale callback firing
  // after a newer query's subscription is already active).
  let docPickerOpen = $state(false);
  let docQuery = $state("");
  let docHits = $state<WireSearchHit[]>([]);
  $effect(() => {
    if (!docPickerOpen) { docHits = []; return; }
    const q = docQuery.trim();
    if (!q) { docHits = []; return; }
    let handle: SubscriptionHandle | null = null;
    let cancelled = false;
    void ctx
      .searchDocuments(q, { limit: 20 }, (hits: WireSearchHit[]) => {
        if (cancelled) return;
        docHits = hits;
      })
      .then((h) => { if (cancelled) h.unsubscribe(); else handle = h; })
      .catch(() => { /* no transport: leave last hits, re-subscribe on next keystroke */ });
    return () => { cancelled = true; handle?.unsubscribe(); };
  });

  /** Display label for a search hit: a token resolves through its linked/embedded actor (the
   * same `resolveTokenActor`/`actorDisplayName` read-through every other actor/token consumer
   * uses), any other document falls back to its envelope `name`.
   * @param doc The candidate document/token.
   * @returns The label to both display in the picker and capture into the inserted span.
   * @example
   * ```
   * // internal; used by the picker's result list and insertDocLink
   * declare const doc: WireDocument;
   * docLinkLabel(doc);
   * ```
   */
  function docLinkLabel(doc: WireDocument): string {
    if (doc.doc_type === "token") {
      const eff = resolveTokenActor(doc, ctx.documents);
      return eff ? actorDisplayName(eff) : ((doc.name ?? "").trim() || doc.id.slice(0, 8));
    }
    return (doc.name ?? "").trim() || `${doc.doc_type} ${doc.id.slice(0, 8)}`;
  }

  /** Inserts a `[[doc:<id>|<label>]]`/`[[token:<id>|<label>]]` span at the textarea's cursor
   * position and closes the picker. Strips every `[`/`]`/`|` from the label before building the
   * span — a document/token's authored name is free text, and any of these three characters is
   * part of `scan_body`'s grammar: an unstripped `[` desyncs its bracket-depth tracking (the
   * span's own closing `]]` can be silently consumed as an ordinary depth-decrement instead of
   * recognized as the terminator), and an unstripped `]]` terminates the span early outright —
   * both corrupt the grammar, not just cosmetically. Falls back to the id's first 8 characters
   * when stripping leaves the label empty (a name consisting only of these characters, or a
   * blank/absent name `docLinkLabel` didn't already substitute a fallback for) — an empty label
   * is `RollError::MalformedDocLink` server-side, rejecting the WHOLE message, not just the link.
   * @param doc The picked document/token.
   * @example
   * ```
   * // internal; wired to each result row's click handler
   * declare const doc: WireDocument;
   * insertDocLink(doc);
   * ```
   */
  function insertDocLink(doc: WireDocument): void {
    const label = docLinkLabel(doc).replace(/[[\]|]/g, "").trim() || doc.id.slice(0, 8);
    const span = doc.doc_type === "token" ? `[[token:${doc.id}|${label}]]` : `[[doc:${doc.id}|${label}]]`;
    const el = textarea;
    const start = el?.selectionStart ?? value.length;
    const end = el?.selectionEnd ?? value.length;
    value = value.slice(0, start) + span + value.slice(end);
    docPickerOpen = false;
    docQuery = "";
    docHits = [];
    const caret = start + span.length;
    queueMicrotask(() => {
      autoGrow();
      el?.focus();
      el?.setSelectionRange(caret, caret);
    });
  }

  // The server's player-presentable rejection reason for the last send, shown
  // inline so a refused message does not vanish silently. Already classified
  // server-side (authorization/existence/internal errors are generic there).
  let errorMsg = $state<string | null>(null);

  const trimmed = $derived(value.trim());
  // Cap/counter/send-gating derive from the TRIMMED length, matching what send() actually
  // transmits and what `handle_send_message` validates against `MAX_MESSAGE_CHARS`.
  // Known, fail-safe divergence: JS .length counts UTF-16 code units
  // while the server counts Unicode scalar values (chars().count()) — a surrogate-pair
  // (astral-plane) character counts as 2 toward the client's length but 1 toward the
  // server's, so the client can only over-block near the cap, never under-block; this
  // asymmetry is safe.
  const overLimit = $derived(trimmed.length > MAX_MESSAGE_CHARS);
  const showCounter = $derived(trimmed.length > COUNTER_THRESHOLD);
  const canSend = $derived(trimmed.length > 0 && !overLimit);

  const placeholder = $derived(audience.kind === "gm_only" ? t("chat.composer.placeholderGm") : t("chat.composer.placeholder", { name: placeholderName }));

  /** Auto-grow: reset height before measuring scrollHeight, or the textarea can never shrink
   * back down after a multi-line message is cleared.
   * @example
   * ```
   * // internal; called after every input and after a successful send
   * autoGrow();
   * ```
   */
  function autoGrow(): void {
    if (!textarea) return;
    textarea.style.height = "auto";
    textarea.style.height = `${textarea.scrollHeight}px`;
  }

  /** Sends the trimmed draft, clears the input optimistically, and surfaces a server
   * rejection inline via `errorMsg` instead of the message vanishing.
   * @example
   * ```
   * // internal; wired to the send button and Enter (see onKeydown)
   * send();
   * ```
   */
  function send(): void {
    if (!canSend) return;
    errorMsg = null;
    // /-commands (e.g. "/roll 1d6") ride verbatim — the server (chat::parse_command) is the
    // sole parser that decides what a command MEANS (MessageKind, whisper targets); this
    // composer never inspects or branches on content shape before sending. (MessageCard's
    // own ROLL_COMMAND_PREFIXES strips a prefix purely for DISPLAY of an already-server-
    // classified roll message — a separate, read-only concern from parsing at send time.)
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

  /** Clears a stale rejection notice as soon as the author starts a fresh message, and
   * re-measures the textarea's height.
   * @example
   * ```
   * // internal; wired to oninput
   * onInput();
   * ```
   */
  function onInput(): void {
    errorMsg = null;
    autoGrow();
  }

  /** Enter sends (unless an IME composition is in progress or Shift is held, in which case
   * Shift+Enter falls through to the textarea's default newline insertion).
   * @param e The keyboard event.
   * @example
   * ```
   * // internal; wired to onkeydown
   * onKeydown(new KeyboardEvent("keydown", { key: "Enter" }));
   * ```
   */
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
      <option value={actor.id}>{actorDisplayName({ name: actor.name, displayName: (actor.engine as { /** The actor's authored display name, if any. */ displayName?: string } | undefined)?.displayName })}</option>
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
  <button type="button" data-testid="doc-link-trigger" title={t("chat.composer.insertDocLink")} onclick={() => (docPickerOpen = !docPickerOpen)}>@doc</button>
  <button type="button" onclick={send} disabled={!canSend}>{t("chat.composer.send")}</button>
</div>
{#if docPickerOpen}
  <div class="doc-picker">
    <label class="visually-hidden" for="chat-composer-doc-search">{t("chat.composer.insertDocLink")}</label>
    <input id="chat-composer-doc-search" type="text" placeholder={t("chat.composer.docSearchPlaceholder")} bind:value={docQuery} />
    <ul class="doc-picker-results">
      {#each docHits as hit (hit.document.id)}
        <li>
          <button type="button" onclick={() => insertDocLink(hit.document)}>{docLinkLabel(hit.document)}</button>
        </li>
      {/each}
    </ul>
  </div>
{/if}
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
  .doc-picker {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin-top: var(--space-1);
    padding: var(--space-1);
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
  }
  .doc-picker input {
    min-height: 44px;
    padding: var(--space-1);
  }
  .doc-picker-results {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 12em;
    overflow-y: auto;
  }
  .doc-picker-results button {
    width: 100%;
    min-height: 44px;
    text-align: left;
    padding: var(--space-1);
  }
</style>
