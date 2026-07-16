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
    type RollOutcome,
  } from "@shadowcat/core";

  type RollEmbedSegment = Extract<ChatSegment, { kind: "roll_embed" }>;
  type RollButtonSegment = Extract<ChatSegment, { kind: "roll_button" }>;

  let { message, showChannel }: { message: WireDocument; showChannel: boolean } = $props();

  const ctx = getAppContext();
  const t = ctx.t;

  // Fail-closed body parse (chat skill): a malformed/foreign-shaped `system` body renders
  // nothing rather than a partially-broken card.
  const sys = $derived(parseMessageEngine(message));

  const authorName = $derived(sys ? (ctx.members.get(sys.user_owner) ?? sys.user_owner.slice(0, 8)) : "");

  // Actor attribution: wraps the ActorOwnerRef in the same read-through every other
  // actor/token consumer uses (resolveTokenActor + actorDisplayName), so the OwnerOrGm
  // name-redaction and dangling-reference fail-closed behavior are inherited, not
  // reimplemented here. `actor_owner.kind === "actor"` has no token to read, so a synthetic
  // link-mode wrapper stands in for one; `token_instance` resolves the real placed token.
  const actorName = $derived.by((): string | null => {
    const owner = sys?.actor_owner;
    if (!owner) return null;
    return resolveActorOwnerName(owner);
  });

  // §5.4: the actor name becomes an openDocument link when the referenced document is
  // present in the per-recipient optimistic store (⇒ this recipient has READ — server-side
  // redaction withholds it otherwise). A `token_instance` ref opens the token (its embedded
  // actor); an `actor` ref opens the actor doc. Absent ⇒ plain text, no link.
  const actorOpenRef = $derived.by((): { tokenId: string } | { docId: string } | null => {
    const owner = sys?.actor_owner;
    if (!owner) return null;
    if (owner.kind === "actor") return ctx.documents.get(owner.actor_id) ? { docId: owner.actor_id } : null;
    return ctx.documents.get(owner.token_id) ? { tokenId: owner.token_id } : null;
  });

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
   * the card. */
  function hostOf(url: string): string {
    try {
      return new URL(url).host;
    } catch {
      return url;
    }
  }

  /** The clickable href for a `link_preview` card, or `undefined` to render it non-clickable.
   * The server only ever stores an `http`/`https` preview URL (fetch_preview's scheme guard),
   * but the card independently re-checks the scheme rather than trust that invariant across the
   * boundary — a stored `javascript:`/`data:` URL (from any future path bypassing fetch_preview,
   * or a serialization bug) must never become a live anchor, since Svelte escapes attribute
   * VALUES but does not filter URL schemes. */
  function safeHref(url: string): string | undefined {
    try {
      const scheme = new URL(url).protocol;
      return scheme === "http:" || scheme === "https:" ? url : undefined;
    } catch {
      return undefined;
    }
  }

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

  /** Concatenates `text` segments only — the edit-prefill + roll-shell source of truth. */
  function textOf(content: (ChatSegment | UnknownSegment)[]): string {
    return content.filter(isKnownSegment).filter((s) => s.kind === "text").map((s) => (s as Extract<ChatSegment, { kind: "text" }>).text).join("");
  }

  // The command tokens `chat::parse_command` accepts (src/server/src/chat/commands.rs) —
  // exact, case-sensitive match. A bare `/NdM` shorthand matches neither prefix and is
  // displayed verbatim (leading slash included).
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
  // roll_embed (server invariant, chat spec §7: a roll message's content is exactly one
  // RollEmbed). The length check runs against `sys.content` (raw, pre-filter) rather than the
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

  function keptValues(outcome: RollOutcome): string {
    return outcome.records.filter((r) => r.kept).map((r) => String(r.value)).join(", ");
  }

  /** Native `title` tooltip for an inline roll chip: formula + kept die values (v1, no
   * rich popover — spec §7/§10). */
  function inlineRollTitle(s: RollEmbedSegment): string {
    return `${s.formula}: ${keptValues(s.outcome)}`;
  }

  /** Roll-button click: a fresh, public, sender-attributed `/roll` on the carrying
   * message's channel — never re-executes the carrying message's own roll (spec §2/§7). */
  function sendRollButton(s: RollButtonSegment): void {
    if (!sys) return;
    ctx.chat.send({ channel: sys.channel, content: `/roll ${s.formula}` });
  }

  const canModerate = $derived(!!sys && (sys.user_owner === ctx.selfId || ctx.role === "gm"));

  let editing = $state(false);
  let draft = $state("");

  function startEdit(): void {
    if (!sys) return;
    draft = sys.source ?? textOf(sys.content);
    editing = true;
  }
  function saveEdit(): void {
    ctx.chat.edit(message.id, draft);
    editing = false;
  }
  function cancelEdit(): void {
    editing = false;
  }
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
                <span class="roll-chip" title={inlineRollTitle(s)}>{s.outcome.successes ?? s.outcome.total}</span>
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
  .roll-chip {
    display: inline-flex;
    padding: 0 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    font-weight: 700;
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
