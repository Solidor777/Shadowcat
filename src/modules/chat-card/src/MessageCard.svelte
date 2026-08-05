<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import {
    parseMessageEngine,
    isKnownSegment,
    resolveTokenActor,
    actorDisplayName,
    type ChatSegment,
    type UnknownSegment,
    type WireActorOwnerRef,
    type WireDocument,
  } from "@shadowcat/core";
  import RollTooltip from "./RollTooltip.svelte";

  type RollEmbedSegment = Extract<ChatSegment, { kind: "roll_embed" }>;
  type RollButtonSegment = Extract<ChatSegment, { kind: "roll_button" }>;

  let { message, showChannel }: { message: WireDocument; showChannel: boolean } = $props();

  const ctx = getAppContext();
  const t = ctx.t;

  // Fail-closed body parse: `parseMessageEngine` (src/client/core/src/chat-docs.ts:164-168)
  // returns null on a wrong doc_type or ANY schema-mismatched `engine` body, so a
  // malformed/foreign-shaped body renders nothing rather than a partially-broken card.
  const sys = $derived(parseMessageEngine(message));

  const authorName = $derived(sys ? (ctx.members.get(sys.user_owner) ?? sys.user_owner.slice(0, 8)) : "");

  // Actor attribution: wraps the ActorOwnerRef in the same read-through every other
  // actor/token consumer uses (resolveTokenActor + actorDisplayName), so the OwnerOrGm
  // name-redaction and dangling-reference fail-closed behavior are inherited, not
  // reimplemented here — true for BOTH branches: `actor_owner.kind === "actor"` has no token
  // to read, so a synthetic link-mode wrapper stands in for one (its `overrides: {}` is a
  // verified no-op — `resolveTokenActor`/`project`, src/client/core/src/actor.ts:93-103 and
  // :36-50, read only `engine.actor_id`/`engine.overrides` off the wrapper, and every
  // `overrides?.X` on an empty object falls through to the real actor's own field); a
  // `token_instance` ref resolves the real placed token through the identical function,
  // unmodified.
  const actorName = $derived.by((): string | null => {
    const owner = sys?.actor_owner;
    if (!owner) return null;
    return resolveActorOwnerName(owner);
  });

  // The actor name becomes an openDocument link when the referenced document is present in
  // the per-recipient OPTIMISTIC store (`ctx.documents` is the OptimisticClient view —
  // src/client/core/src/optimistic.ts — base confirmed by the server, plus this client's own
  // pending intents): normally presence implies READ, since server-side redaction withholds
  // an unauthorized doc from `base` entirely, EXCEPT a doc present only via this client's own
  // not-yet-confirmed `applyIntent` prediction (e.g. an actor this user just tried to create)
  // hasn't cleared that check yet — the link briefly reflects the local guess until the
  // confirm/reject echo settles. A `token_instance` ref opens the token (its embedded actor);
  // an `actor` ref opens the actor doc. Absent ⇒ plain text, no link.
  const actorOpenRef = $derived.by((): { tokenId: string } | { docId: string } | null => {
    const owner = sys?.actor_owner;
    if (!owner) return null;
    if (owner.kind === "actor") return ctx.documents.get(owner.actor_id) ? { docId: owner.actor_id } : null;
    return ctx.documents.get(owner.token_id) ? { tokenId: owner.token_id } : null;
  });

  /** Resolves an `ActorOwnerRef` to its display name via the shared
   * `resolveTokenActor`/`actorDisplayName` read-through (see `actorName` above for why both
   * branches inherit the same redaction/fail-closed behavior).
   * @param owner The message's `actor_owner` reference.
   * @returns The resolved display name, or `null` for a dangling/unresolvable reference.
   * @example
   * ```
   * // internal; call sites use the `actorName` derived above
   * resolveActorOwnerName({ kind: "actor", actor_id: "00000000-0000-0000-0000-000000000001" });
   * ```
   */
  function resolveActorOwnerName(owner: WireActorOwnerRef): string | null {
    if (owner.kind === "actor") {
      const synthetic = { engine: { actor_id: owner.actor_id, overrides: {} } } as unknown as WireDocument;
      const eff = resolveTokenActor(synthetic, ctx.documents);
      return eff ? actorDisplayName(eff) : null;
    }
    const token = ctx.documents.get(owner.token_id);
    if (!token) return null;
    const eff = resolveTokenActor(token, ctx.documents);
    return eff ? actorDisplayName(eff) : null;
  }

  /** Host caption for a `link_preview` card. Never throws on a malformed `url` — a preview is
   * server-fetched and validated at ingest, but the client mirror trusts nothing about the
   * stored string's shape, so a bad URL degrades to showing the raw string instead of crashing
   * the card.
   * @param url The stored preview URL.
   * @returns The URL's host, or the raw `url` string when it fails to parse.
   * @example
   * ```
   * hostOf("https://example.test/x"); // "example.test"
   * hostOf("not a url"); // "not a url"
   * ```
   */
  function hostOf(url: string): string {
    try {
      return new URL(url).host;
    } catch {
      return url;
    }
  }

  /** The clickable href for a `link_preview` card, or `undefined` to render it non-clickable.
   * The server only ever stores an `http`/`https` preview URL: `validate_url`
   * (src/server/src/chat/link_preview.rs:705-728, re-run on every redirect hop) rejects any
   * other scheme before a `LinkPreview` is ever constructed (`:684-691`). This function does
   * not trust that invariant across the wire boundary and independently re-checks the scheme —
   * a stored `javascript:`/`data:` URL (from a future path bypassing `fetch_preview`, or a
   * serialization bug) must never become a live anchor: Svelte escapes an interpolated
   * attribute VALUE (preventing quote/attribute breakout) but performs no scheme filtering on a
   * dynamic `href` at runtime, so this check is the only thing standing between a bad stored
   * URL and a live link.
   * @param url The stored preview URL.
   * @returns `url` when its scheme is `http:`/`https:`, else `undefined`.
   * @example
   * ```
   * safeHref("https://example.test/x"); // "https://example.test/x"
   * safeHref("javascript:alert(1)"); // undefined
   * ```
   */
  function safeHref(url: string): string | undefined {
    try {
      const scheme = new URL(url).protocol;
      return scheme === "http:" || scheme === "https:" ? url : undefined;
    } catch {
      return undefined;
    }
  }

  /** `HH:MM` in the VIEWER's local timezone (`Date.getHours`/`getMinutes` read local time,
   * not UTC) — a reader comparing this against a server-side UTC timestamp elsewhere must
   * account for the offset. `timeTitle` below renders the same instant via `toLocaleString`,
   * which also defaults to local time, so the two never disagree on timezone, only on format
   * verbosity.
   * @param ms The message's `created_at` epoch milliseconds.
   * @returns A zero-padded `HH:MM` string.
   * @example
   * ```
   * formatTime(0); // "00:00" in UTC; the viewer's local offset shifts this
   * ```
   */
  function formatTime(ms: number): string {
    const d = new Date(ms);
    return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
  }
  const timeLabel = $derived(formatTime(message.created_at));
  const timeTitle = $derived(new Date(message.created_at).toLocaleString());

  const whisperNames = $derived.by((): string => {
    if (sys?.audience.kind !== "whisper") return "";
    return sys.audience.recipients.map((r: string) => ctx.members.get(r) ?? r.slice(0, 8)).join(", ");
  });

  /** Concatenates `text` segments only — the edit-prefill + roll-shell source of truth.
   * @param content The message's segment list (raw, including any unknown segments).
   * @returns The concatenated text of every known `text` segment, in order.
   * @example
   * ```
   * textOf([{ kind: "text", text: "hi" }]); // "hi"
   * ```
   */
  function textOf(content: (ChatSegment | UnknownSegment)[]): string {
    return content.filter(isKnownSegment).filter((s) => s.kind === "text").map((s) => (s as Extract<ChatSegment, { kind: "text" }>).text).join("");
  }

  // The two explicit roll-command prefixes `chat::parse_command` accepts via its
  // `for tok in ["/roll ", "/r "]` loop (src/server/src/chat/commands.rs:39-46) — exact,
  // case-sensitive match, same trailing space on each. The server ALSO accepts a bare `/NdM`
  // shorthand as a separate roll-triggering form (commands.rs:48-56, matched via
  // `strip_prefix('/')` + `is_dice_shorthand`, not this loop) — omitted here on purpose, so a
  // bare `/NdM` matches neither entry and `rollFormula` below displays it verbatim (leading
  // slash included) rather than stripping a prefix. This is a display-scope statement about
  // THIS client's formula rendering, not a claim that the server ignores `/NdM` — it does not.
  const ROLL_COMMAND_PREFIXES = ["/roll ", "/r "];

  /** Roll-pending display formula. `source` holds the FULL raw author input (including any
   * command token) so a markdown/html-enabled world's `sanitize()` wrapping the body into a
   * single `Segment::Html` never loses the formula — `textOf` alone would read empty here. */
  const rollFormula = $derived.by((): string => {
    if (!sys) return "";
    const src = sys.source ?? textOf(sys.content);
    for (const prefix of ROLL_COMMAND_PREFIXES) {
      if (src.startsWith(prefix)) return src.slice(prefix.length);
    }
    return src;
  });

  // Block form only when the WHOLE RAW content is exactly one segment and that segment is a
  // roll_embed (server invariant: a successful roll's content is built as exactly one
  // RollEmbed — src/server/src/chat/mod.rs:651-654 constructs it, pinned by the test at
  // :3499-3513). The length check runs against `sys.content` (raw, pre-filter) rather than the
  // known-segment-filtered list — filtering first would let an extra UNKNOWN segment silently
  // vanish and the lone roll_embed still render as a block, contradicting this guard's own
  // "additional/other segments fall back to the pending shell" intent.
  const rollBlock = $derived.by((): RollEmbedSegment | null => {
    if (!sys || sys.kind !== "roll") return null;
    if (sys.content.length !== 1) return null;
    const known = sys.content.filter(isKnownSegment);
    if (known.length === 1 && known[0].kind === "roll_embed") return known[0] as RollEmbedSegment;
    return null;
  });

  /** Roll-button click: a fresh, public, sender-attributed `/roll` on the carrying message's
   * channel — never re-executes the carrying message's own roll
   * (docs/superpowers/specs/2026-07-13-m11d-2-dice-chat-wire-design.md §2 item 3, §7).
   * @param s The clicked `roll_button` segment.
   * @example
   * ```
   * sendRollButton({ kind: "roll_button", formula: "1d20", label: null });
   * ```
   */
  function sendRollButton(s: RollButtonSegment): void {
    if (!sys) return;
    ctx.chat.send({ channel: sys.channel, content: `/roll ${s.formula}` });
  }

  // Advisory only — the server independently re-authorizes every edit/delete against its own
  // owner-or-GM check (src/server/src/chat/mod.rs:860-863 for edit, :1018-1020 for delete);
  // this gate only decides whether to SHOW the actions, never whether one succeeds.
  const canModerate = $derived(!!sys && (sys.user_owner === ctx.selfId || ctx.role === "gm"));

  let editing = $state(false);
  let draft = $state("");

  /** Enters edit mode, prefilling the draft from the raw author input (falling back to the
   * rendered text when `source` is absent).
   * @example
   * ```
   * // internal; called from the edit action button
   * startEdit();
   * ```
   */
  function startEdit(): void {
    if (!sys) return;
    draft = sys.source ?? textOf(sys.content);
    editing = true;
  }
  /** Submits the edit draft via `ctx.chat.edit` and exits edit mode optimistically; a server
   * rejection surfaces separately through chat's shared `ChatError` seam, not through this
   * function.
   * @example
   * ```
   * // internal; called from the edit box's Save button
   * saveEdit();
   * ```
   */
  function saveEdit(): void {
    ctx.chat.edit(message.id, draft);
    editing = false;
  }
  /** Discards the draft and exits edit mode without sending anything.
   * @example
   * ```
   * // internal; called from the edit box's Cancel button
   * cancelEdit();
   * ```
   */
  function cancelEdit(): void {
    editing = false;
  }
  /** Sends chat's dedicated delete frame (`ctx.chat.delete`) after a native confirm dialog.
   * The server applies this as a soft-tombstoning `Operation::Update` on `/engine`
   * (src/server/src/chat/mod.rs:1033), published under `WriteOrigin::ServerMessageRevision`
   * (`:1042`) — never a hard `Operation::Delete`; the doc stays in the sequenced log at its
   * original seq. A client-authored hard delete of a `message` doc is independently rejected
   * at both transport ingress points (`ops_target_message`, src/server/src/chat/mod.rs:78-83),
   * so this frame is the only way a message is ever removed.
   * @example
   * ```
   * // internal; called from the delete action button
   * doDelete();
   * ```
   */
  function doDelete(): void {
    if (window.confirm(t("chat.deleteConfirm"))) ctx.chat.delete(message.id);
  }
</script>

{#if sys}
  <article class="card" class:emote={sys.kind === "emote"} class:system={sys.kind === "system"}>
    <header class="header">
      <span class="author">{authorName}</span>
      {#if actorName}
        {#if actorOpenRef}
          <button type="button" class="actor-name link" onclick={() => ctx.openDocument(actorOpenRef)} aria-label={t("chat.openActor", { name: actorName })}>({actorName})</button>
        {:else}
          <em class="actor-name">({actorName})</em>
        {/if}
      {/if}
      <time datetime={new Date(message.created_at).toISOString()} title={timeTitle}>{timeLabel}</time>
      {#if showChannel}
        <span class="chip channel">{sys.channel}</span>
      {/if}
      {#if sys.edited_at}
        <span class="chip edited">{t("chat.edited")}</span>
      {/if}
      {#if sys.audience.kind === "whisper"}
        <span class="chip whisper">{t("chat.whisperTo", { names: whisperNames })}</span>
      {/if}
      {#if sys.audience.kind === "gm_only"}
        <span class="chip gm">{t("chat.gmBadge")}</span>
      {/if}
      {#if sys.kind === "system"}
        <span class="chip system-badge">{t("chat.systemBadge")}</span>
      {/if}
    </header>

    {#if sys.deleted_at}
      <p class="tombstone">{t("chat.deleted")}</p>
    {:else if editing}
      <div class="edit-box">
        <textarea aria-label={t("chat.edit")} bind:value={draft}></textarea>
        <div class="edit-actions">
          <button type="button" onclick={saveEdit}>{t("chat.save")}</button>
          <button type="button" onclick={cancelEdit}>{t("chat.cancel")}</button>
        </div>
      </div>
    {:else}
      <div class="body">
        {#if rollBlock}
          <div class="roll-block">
            <div class="roll-formula" aria-label={t("chat.roll.formula")}>{rollBlock.formula}</div>
            {#if rollBlock.outcome.successes != null}
              <div class="roll-result">
                <span class="roll-successes">{t("chat.roll.successes", { n: rollBlock.outcome.successes })}</span>
                {#if rollBlock.outcome.tier_label}
                  <span class="roll-tier">{rollBlock.outcome.tier_label}</span>
                {:else if rollBlock.outcome.pass != null}
                  <span class="roll-pass" class:pass={rollBlock.outcome.pass} class:fail={!rollBlock.outcome.pass}>
                    {rollBlock.outcome.pass ? t("chat.roll.pass") : t("chat.roll.fail")}
                  </span>
                {/if}
              </div>
            {:else}
              <div class="roll-result roll-total">{rollBlock.outcome.total}</div>
            {/if}
            <div class="roll-dice">
              {#each rollBlock.outcome.records as r, i (i)}
                <span
                  class="die-chip"
                  class:dropped={!r.kept}
                  class:crit-success={r.crit_success}
                  class:crit-fail={r.crit_fail}
                >
                  <span class="die-value">{r.value}</span>
                  {#if r.label}<span class="die-label">{r.label}</span>{/if}
                  {#if r.symbols.length > 0}<span class="die-symbols">{r.symbols.join(" ")}</span>{/if}
                </span>
              {/each}
              {#each rollBlock.outcome.labeled_consts as c, i (i)}
                <span class="die-chip const-chip">
                  <span class="die-value">{c.value}</span>
                  {#if c.label}<span class="die-label">{c.label}</span>{/if}
                </span>
              {/each}
            </div>
            {#if rollBlock.outcome.positive_counter !== 0 || rollBlock.outcome.negative_counter !== 0}
              <div class="roll-counters">
                {#if rollBlock.outcome.positive_counter !== 0}
                  <span class="counter positive">+{rollBlock.outcome.positive_counter}</span>
                {/if}
                {#if rollBlock.outcome.negative_counter !== 0}
                  <span class="counter negative">-{rollBlock.outcome.negative_counter}</span>
                {/if}
              </div>
            {/if}
            {#if Object.keys(rollBlock.outcome.symbol_counts).length > 0}
              <div class="roll-symbol-counts">
                {#each Object.entries(rollBlock.outcome.symbol_counts) as [symbol, count] (symbol)}
                  <span class="symbol-count">{symbol}: {count}</span>
                {/each}
              </div>
            {/if}
          </div>
        {:else if sys.kind === "roll"}
          <p class="roll-pending">{t("chat.rollPending", { formula: rollFormula })}</p>
        {:else}
          <p>
            {#if sys.kind === "emote"}
              <span class="seg-text">{authorName} </span>
            {/if}
            {#each sys.content.filter(isKnownSegment) as s, i (i)}
              {#if s.kind === "text"}
                <span class="seg-text">{s.text}</span>
              {:else if s.kind === "html"}
                <!-- INVARIANT: sanitized_html is ammonia-cleaned by the server's chat::sanitize —
                the ONLY string this app may ever pass to {@html}. Every other segment kind
                (text, roll_embed, roll_button, link_preview) renders via escaped interpolation
                only; link_preview in particular is a server-fetched title/description/url and
                is rendered with plain `{...}` text bindings, never innerHTML. -->
                <span class="seg-html">{@html s.sanitized_html}</span>
              {:else if s.kind === "roll_embed"}
                <RollTooltip outcome={s.outcome} />
              {:else if s.kind === "roll_button"}
                <button type="button" class="roll-btn" onclick={() => sendRollButton(s)}>
                  {s.label ?? s.formula}
                </button>
              {:else if s.kind === "link_preview"}
                <!-- Server-fetched preview (SSRF-guarded, M11d-3). The client NEVER fetches
                `s.url` or any remote resource — only stored title/description/url strings are
                rendered, all as escaped text. No <img>: an <img src> would make the viewer's
                browser fetch a remote resource, leaking their IP to a URL an attacker chose. -->
                <a
                  class="link-preview"
                  href={safeHref(s.url)}
                  target="_blank"
                  rel="noopener noreferrer nofollow"
                >
                  <span class="link-preview-title">{s.title}</span>
                  <span class="link-preview-description">{s.description}</span>
                  <span class="link-preview-host">{hostOf(s.url)}</span>
                </a>
              {/if}
            {/each}
          </p>
        {/if}
      </div>

      {#if canModerate}
        <div class="actions">
          <button type="button" onclick={startEdit}>{t("chat.edit")}</button>
          <button type="button" onclick={doDelete}>{t("chat.delete")}</button>
        </div>
      {/if}
    {/if}
  </article>
{/if}

<style lang="scss">
  .card {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-1);
  }
  .header {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: var(--space-1);
  }
  .author {
    font-weight: 700;
  }
  .chip {
    font-size: 0.85em;
    padding: 0 4px;
    border-radius: var(--radius-1);
    border: 1px solid var(--border);
  }
  .tombstone {
    font-style: italic;
    opacity: 0.7;
  }
  .actor-name.link {
    font-style: italic;
    background: none;
    border: none;
    padding: 0;
    color: var(--accent);
    cursor: pointer;
    font: inherit;
  }
  .actor-name.link:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  // Single hook for emote styling — anchored on .card (the class:emote binding), not a
  // second class on the inner <p>, so there is one source of truth for "this is an emote".
  .card.emote .body p {
    font-style: italic;
  }
  .roll-pending {
    font-family: monospace;
  }
  .roll-block {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-1);
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
  }
  .roll-formula {
    font-family: monospace;
    opacity: 0.8;
  }
  .roll-result {
    display: flex;
    align-items: baseline;
    gap: var(--space-1);
    font-size: 1.4em;
    font-weight: 700;
  }
  .roll-pass.pass {
    color: var(--success, seagreen);
  }
  .roll-pass.fail {
    color: var(--danger, crimson);
  }
  .roll-dice {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }
  .die-chip {
    display: inline-flex;
    align-items: baseline;
    gap: 4px;
    padding: 0 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
  }
  .die-chip.dropped {
    opacity: 0.5;
    text-decoration: line-through;
  }
  .die-chip.crit-success {
    border-color: var(--success, seagreen);
  }
  .die-chip.crit-fail {
    border-color: var(--danger, crimson);
  }
  .die-label,
  .die-symbols {
    font-size: 0.85em;
    opacity: 0.8;
  }
  .roll-counters,
  .roll-symbol-counts {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    font-size: 0.9em;
  }
  .counter.positive {
    color: var(--success, seagreen);
  }
  .counter.negative {
    color: var(--danger, crimson);
  }
  .roll-btn {
    // Touch floor: matches .actions button (spec §7 — 44px target).
    min-height: 44px;
    min-width: 44px;
    padding: 0 var(--space-1);
  }
  .card.system {
    opacity: 0.75;
    font-style: italic;
  }
  .chip.system-badge {
    font-style: normal;
    opacity: 0.9;
  }
  .seg-text {
    // Preserves author-typed newlines in a plain-text segment; without this a multi-line
    // message collapses to one visual line despite the \n surviving in the DOM text node.
    white-space: pre-wrap;
  }
  .actions {
    display: flex;
    gap: var(--space-1);
  }
  .actions button {
    // Touch floor: these buttons are permanently visible on touch devices (no
    // hover concept to reveal them), so they must clear the 44px minimum even
    // though they render as small pill-style controls on desktop.
    min-height: 44px;
    min-width: 44px;
    padding: 0 var(--space-1);
  }
  // Hover/focus-reveal on hover-capable devices, always-visible on touch (no hover concept
  // to reveal on). Uses opacity/pointer-events, never visibility/display, so the buttons
  // stay in the tab order — Tab-focusing one triggers :focus-within on .card and reveals it.
  @media (hover: hover) {
    .actions {
      opacity: 0;
      pointer-events: none;
    }
    .card:hover .actions,
    .card:focus-within .actions {
      opacity: 1;
      pointer-events: auto;
    }
  }
  .edit-box {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .edit-box textarea {
    min-height: 44px;
  }
  .edit-actions {
    display: flex;
    gap: var(--space-1);
  }
  // Spec §5: images size to the card and sit on their own line.
  .seg-html :global(img) {
    max-width: 100%;
    display: block;
  }
  // Link-preview card (spec §7, M11d-3). No <img> — server-fetched title/description/host
  // only, all escaped text; the whole card is the link (44px touch floor on the anchor itself).
  .link-preview {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-height: 44px;
    padding: var(--space-1);
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    text-decoration: none;
    color: inherit;
  }
  .link-preview-title {
    font-weight: 700;
  }
  .link-preview-description {
    opacity: 0.75;
    // Clamps to ~2 lines rather than letting a long server-fetched description balloon the
    // card's height in the message list.
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .link-preview-host {
    font-size: 0.85em;
    opacity: 0.6;
  }
</style>
