<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { actorDisplayName, resolveTokenActor, MAX_MESSAGE_CHARS, CHAT_SETTINGS_DOC_TYPE, type WireActorOwnerRef, type WireAudience, type WireDocument, type WireSearchHit, type SubscriptionHandle, type ChatSettingsEngine } from "@shadowcat/core";

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

  // The sticky speak-as selection lives in session state (`ctx.speakAs`, ui-kit's `SpeakAs`) so
  // every roll-producing surface — this composer and the chat card's roll buttons — resolves
  // the same binding; dice references in a roll resolve server-side against it. Empty string
  // is the sentinel for "Myself" (the default, no-attribution option).

  // Prunes a dangling selection: if the selected actor leaves `speakableActors`
  // (deleted, or ownership transferred away from the current user), the select
  // would otherwise render blank (selectedIndex -1) while sends kept attaching
  // the now-nonexistent actor_id. Resets to "" (Myself) whenever the current
  // selection no longer resolves to a live, speakable actor doc.
  $effect(() => {
    if (ctx.speakAs.actorId && !speakableActors.some((a) => a.id === ctx.speakAs.actorId)) {
      ctx.speakAs.actorId = "";
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

  // Whether the world's chat-settings singleton has images enabled — read the
  // same way `hyperlinks`/`markdown`/etc. resolve server-side (absent = off,
  // `ChatContentPolicy::images`'s `unwrap_or(false)`), reactive to the same
  // store subscriber bridge every other reactive read in this component uses,
  // so a GM toggling the setting live shows/hides the button without a reload.
  const imagesEnabled = $derived.by((): boolean => {
    subscribe();
    const doc = ctx.documents.query(CHAT_SETTINGS_DOC_TYPE)[0];
    return ((doc?.engine as ChatSettingsEngine | undefined)?.images ?? false) === true;
  });

  /** Inserts a `[[asset:<id>|<label>]]` span at the textarea's cursor position, mirroring
   * `insertDocLink`'s span-insertion mechanics. No name lookup surface exists on
   * `AppContext` for a picked asset id, so the label is always the id's first 8
   * hex characters — never free text, so it needs none of `insertDocLink`'s
   * `[`/`]`/`|` stripping (a UUID's hex characters cannot collide with the
   * `scan_body_capped` grammar's delimiters). `[[asset:...]]`'s authority is the id, not the
   * label. A cancelled pick (`null`) is a no-op.
   * @example
   * ```
   * // internal; wired to the "Insert image" button's click handler
   * void insertImage();
   * ```
   */
  async function insertImage(): Promise<void> {
    const id = await ctx.pickAsset({ kind: "image" });
    if (!id) return;
    const label = id.slice(0, 8);
    const span = `[[asset:${id}|${label}]]`;
    const el = textarea;
    const start = el?.selectionStart ?? value.length;
    const end = el?.selectionEnd ?? value.length;
    value = value.slice(0, start) + span + value.slice(end);
    const caret = start + span.length;
    queueMicrotask(() => {
      autoGrow();
      el?.focus();
      el?.setSelectionRange(caret, caret);
    });
  }

  // Roll-mode picker: each entry is a leading command the server's `parse_command` already
  // accepts. Picking one rewrites the visible draft's leading roll command, so the user reads
  // exactly what will be sent and the server stays the only parser of what it means.
  const ROLL_MODES = [
    { command: "/blindroll ", labelKey: "chat.composer.rollModeBlind" },
    { command: "/selfroll ", labelKey: "chat.composer.rollModeSelf" },
  ] as const;
  // Every leading roll command the picker replaces: the picker's own, their aliases, and `/roll`.
  const REPLACEABLE_ROLL_COMMANDS = ["/blindroll ", "/br ", "/selfroll ", "/sr ", "/roll ", "/r "];
  let rollMode = $state("");

  /** Puts `command` at the start of the draft, replacing a roll command the draft already
   * starts with, then returns focus to the textarea and resets the picker to its prompt.
   * @param command One of the picker's `ROLL_MODES` commands, trailing space included.
   * @example
   * ```
   * // internal; wired to the roll-mode select's change handler
   * applyRollMode("/blindroll ");
   * ```
   */
  function applyRollMode(command: string): void {
    if (!command) return;
    let body = value.trimStart();
    const existing = REPLACEABLE_ROLL_COMMANDS.find((c) => body.startsWith(c));
    if (existing) body = body.slice(existing.length);
    value = command + body;
    rollMode = "";
    queueMicrotask(() => {
      autoGrow();
      textarea?.focus();
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

  // In-flight guard: a second Enter/click while the `chat:composer-send` hook chain is still
  // running must not send the draft twice. Checked alongside `canSend` and reflected in the
  // Send button's `disabled`. Scoped to the HOOK's own await, not the wire round-trip: an
  // ordinary accepted chat op carries no ack frame and its promise settles only on the
  // client's silence timeout (`WsClient.trackChatOp`'s multi-second window), so gating on
  // that resolution would leave every message's Send control disabled for the whole timeout
  // window on the common, successful path — see `send`'s own doc for the guard's exact scope.
  let sending = $state(false);

  const placeholder = $derived(audience.kind === "gm_only" ? t("chat.composer.placeholderGm") : t("chat.composer.placeholder", { name: placeholderName }));

  // Speak-as-token indicator: mirrors how the sticky actor `<select>` above surfaces its own
  // selection, but reads the one-shot `ctx.speakAsToken` pending value instead.
  const speakAsTokenName = $derived.by((): string | null => {
    subscribe();
    const id = ctx.speakAsToken.tokenId;
    if (!id) return null;
    const tok = ctx.documents.get(id);
    if (!tok) return null;
    const eff = resolveTokenActor(tok, ctx.documents);
    return eff ? actorDisplayName(eff) : ((tok.name ?? "").trim() || id.slice(0, 8));
  });

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

  /** Identity of the most recent send attempt, claimed when `send` starts — before the
   * `chat:composer-send` hook's await, and kept even if the hook vetoes. A rejection restores its
   * own draft only while its attempt is still the latest one, so an earlier rejection can never
   * overwrite the composer once a later attempt has begun, whether that attempt is still in its
   * hook, was vetoed, or has been dispatched. */
  let latestSend = 0;

  /** Sends the trimmed draft, clears the input optimistically, and surfaces a server
   * rejection inline via `errorMsg` instead of the message vanishing. Before the frame is
   * built, runs the draft through the `chat:composer-send` hook (`ctx.hooks.emitMutate`) —
   * a module-authored system command implementation may rewrite the text or veto the send
   * entirely; this composer applies whichever outcome the hook chain settled on and never
   * inspects the text itself. The optimistic clear only fires when `value` still equals the
   * snapshot taken before that hook's await — a slow hook must not discard text the author
   * typed while it was in flight. INVARIANT: `sending` is scoped to the hook's own await, not
   * to `ctx.chat.send`'s returned promise — an ordinary accepted chat op resolves that promise
   * only on `WsClient.trackChatOp`'s silence timeout (there is no ack frame for a successful
   * send), so awaiting it inside the guard would disable the Send control for that whole
   * timeout window on every ordinary message. The hook call and the frame build run inside a
   * `try`/`finally` that resets `sending` on every exit from THAT region, including a
   * SYNCHRONOUS throw from the hook (a `.finally()` attached only to a promise can never
   * observe a throw that happens before any promise exists to attach to). The dispatched send
   * itself settles independently, fire-and-forget with its own `.catch` surfacing a later
   * rejection via `errorMsg` and restoring `draftAtSend` into the composer — but ONLY when no
   * later send attempt has started (`latestSend`) and the composer is still empty, so neither
   * text the author typed while the rejection was in flight nor a later attempt's state is ever
   * overwritten.
   * @example
   * ```
   * // internal; wired to the send button and Enter (see onKeydown)
   * void send();
   * ```
   */
  async function send(): Promise<void> {
    if (!canSend || sending) return;
    sending = true;
    errorMsg = null;
    const thisSend = ++latestSend;
    // Snapshot the draft actually being sent, taken before the hook's await — compared against
    // `value` after the await to decide whether the optimistic clear is still safe to apply.
    const draftAtSend = value;
    try {
      const outgoing = await ctx.hooks.emitMutate("chat:composer-send", { text: trimmed, cancel: false });
      if (outgoing.cancel) return;
      // History keeps the raw typed text (a `/` command prefix included), not the hook-rewritten
      // or server-sanitized form, so a recall resends exactly what the author typed.
      ctx.uiState.pushChatHistory(draftAtSend.trim());
      historyIndex = -1;
      draft = "";
      // /-commands (e.g. "/roll 1d6") ride verbatim past THIS composer — the server
      // (chat::parse_command) is the sole parser that decides what a command MEANS
      // (MessageKind, whisper targets); a module may still rewrite the text above via the hook
      // (e.g. to implement its own client-only slash command), but whatever text results is
      // never itself inspected or branched on here. (MessageCard's own ROLL_COMMAND_PREFIXES
      // strips a prefix purely for DISPLAY of an already-server-classified roll message — a
      // separate, read-only concern from parsing at send time.)
      const pendingToken = ctx.speakAsToken.consume();
      const actorOwner: WireActorOwnerRef | undefined = pendingToken
        ? { kind: "token_instance", token_id: pendingToken }
        : ctx.speakAs.actorId
          ? { kind: "actor", actor_id: ctx.speakAs.actorId }
          : undefined;
      // Clear the input optimistically; a server rejection surfaces inline via `errorMsg`
      // (correlated by request_id under the seam) instead of the message vanishing. Not
      // awaited here — see the guard's own doc for why blocking on the wire round-trip would
      // disable Send for the ordinary-success silence-timeout window.
      Promise.resolve(
        ctx.chat.send(actorOwner ? { channel, content: outgoing.text, audience, actorOwner } : { channel, content: outgoing.text, audience }),
      ).catch((e: unknown) => {
        errorMsg = e instanceof Error ? e.message : t("chat.composer.sendFailed");
        // Restore the sent draft only if no later send attempt has started and the composer is
        // still empty — text typed since the optimistic clear, or anything a later attempt
        // owns, must never be overwritten by a rejection surfacing for this earlier send.
        if (thisSend === latestSend && value === "") {
          value = draftAtSend;
          queueMicrotask(autoGrow);
        }
      });
      // Only clear a draft the author has not since changed — text typed during the hook's
      // await must survive rather than being silently discarded by this optimistic clear.
      if (value === draftAtSend) {
        value = "";
        queueMicrotask(autoGrow);
      }
    } catch (e: unknown) {
      errorMsg = e instanceof Error ? e.message : t("chat.composer.sendFailed");
    } finally {
      sending = false;
    }
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
    historyIndex = -1;
    autoGrow();
  }

  /** Position in `ctx.uiState.getChatHistory()` of the entry currently recalled into the input;
   * `-1` while the input holds the author's own draft. Not reactive: only the key handler reads it. */
  let historyIndex = -1;
  /** The author's in-progress text, saved when recall starts and restored when the author steps
   * back past the newest entry, so a habitual ArrowUp never discards an unsent draft. */
  let draft = "";
  /** The exact text last written by recall; while `value` still equals it, the input is
   * navigating history rather than being edited. */
  let recalled = "";

  /** Writes `text` into the input, places the caret, and re-measures the height.
   * @param text The text to show.
   * @param caret `"start"` or `"end"`: where the caret lands, so a repeated arrow press keeps navigating.
   * @example
   * ```
   * // internal; used by recallHistory
   * showText("hello", "end");
   * ```
   */
  function showText(text: string, caret: "start" | "end"): void {
    value = text;
    const el = textarea;
    queueMicrotask(() => {
      autoGrow();
      const at = caret === "start" ? 0 : text.length;
      el?.setSelectionRange(at, at);
    });
  }

  /** ArrowUp/ArrowDown history recall over the author's own sent texts, newest first. Recall
   * starts only from a collapsed caret at the very start (up) or very end (down) of the input,
   * so ordinary caret movement through a multi-line draft is untouched; once an entry is
   * recalled and unedited, a single-line entry navigates from any caret position. Stepping down
   * past the newest entry restores the saved draft.
   * @param e The ArrowUp/ArrowDown keydown, `preventDefault`ed only when recall acts.
   * @example
   * ```
   * // internal; called from onKeydown
   * recallHistory(new KeyboardEvent("keydown", { key: "ArrowUp" }));
   * ```
   */
  function recallHistory(e: KeyboardEvent): void {
    const history = ctx.uiState.getChatHistory();
    const navigating = historyIndex >= 0 && value === recalled;
    const el = textarea;
    const collapsed = el !== undefined && el.selectionStart === el.selectionEnd;
    const waived = navigating && !value.includes("\n");
    if (e.key === "ArrowUp") {
      if (history.length === 0 || !(waived || (collapsed && el?.selectionStart === 0))) return;
      e.preventDefault();
      if (navigating) {
        historyIndex = Math.max(0, historyIndex - 1);
      } else {
        draft = value;
        historyIndex = history.length - 1;
      }
      recalled = history[historyIndex];
      showText(recalled, "start");
      return;
    }
    if (!navigating || !(waived || (collapsed && el?.selectionStart === value.length))) return;
    e.preventDefault();
    if (historyIndex >= history.length - 1) {
      historyIndex = -1;
      recalled = "";
      showText(draft, "end");
      draft = "";
      return;
    }
    historyIndex += 1;
    recalled = history[historyIndex];
    showText(recalled, "end");
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
      void send();
    }
    // Shift+Enter falls through to the textarea's default newline insertion.
    if ((e.key === "ArrowUp" || e.key === "ArrowDown") && !e.shiftKey && !e.ctrlKey && !e.altKey && !e.metaKey) {
      recallHistory(e);
    }
  }
</script>

{#if speakAsTokenName}
  <div class="speaking-as">{t("chat.composer.speakingAsToken", { name: speakAsTokenName })}</div>
{/if}
<div class="composer">
  <label class="visually-hidden" for="chat-composer-speak-as">{t("chat.composer.speakAs")}</label>
  <select id="chat-composer-speak-as" bind:value={ctx.speakAs.actorId}>
    <option value="">{t("chat.composer.myself")}</option>
    {#each speakableActors as actor (actor.id)}
      <option value={actor.id}>{actorDisplayName({ name: actor.name, displayName: (actor.engine as { /** The actor's authored display name, if any. */ displayName?: string } | undefined)?.displayName })}</option>
    {/each}
  </select>
  <label class="visually-hidden" for="chat-composer-roll-mode">{t("chat.composer.rollMode")}</label>
  <select id="chat-composer-roll-mode" data-testid="roll-mode" data-tour="chat-composer.roll-mode" bind:value={rollMode} onchange={() => applyRollMode(rollMode)}>
    <option value="">{t("chat.composer.rollMode")}</option>
    {#each ROLL_MODES as mode (mode.command)}
      <option value={mode.command}>{t(mode.labelKey)}</option>
    {/each}
  </select>
  <label class="visually-hidden" for="chat-composer-input">{placeholder}</label>
  <textarea
    id="chat-composer-input"
    data-tour="chat-composer.input"
    bind:this={textarea}
    bind:value
    {placeholder}
    onkeydown={onKeydown}
    oninput={onInput}
    rows="1"
  ></textarea>
  <button type="button" data-testid="doc-link-trigger" data-tour="chat-composer.doc-link" title={t("chat.composer.insertDocLink")} onclick={() => (docPickerOpen = !docPickerOpen)}>@doc</button>
  {#if imagesEnabled}
    <button type="button" data-testid="image-insert" title={t("chat.composer.insertImage")} onclick={() => void insertImage()}>🖼</button>
  {/if}
  <button type="button" onclick={() => void send()} disabled={!canSend || sending}>{t("chat.composer.send")}</button>
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
  .speaking-as {
    margin-bottom: var(--space-1);
    font-size: 0.85em;
    font-style: italic;
    opacity: 0.85;
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
