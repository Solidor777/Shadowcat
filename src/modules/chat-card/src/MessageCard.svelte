<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import {
    parseMessageSystem,
    isKnownSegment,
    resolveTokenActor,
    actorDisplayName,
    type ChatSegment,
    type UnknownSegment,
    type WireActorOwnerRef,
    type WireDocument,
  } from "@shadowcat/core";

  let { message, showChannel }: { message: WireDocument; showChannel: boolean } = $props();

  const ctx = getAppContext();
  const t = ctx.t;

  // Fail-closed body parse (chat skill): a malformed/foreign-shaped `system` body renders
  // nothing rather than a partially-broken card.
  const sys = $derived(parseMessageSystem(message));

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

  function resolveActorOwnerName(owner: WireActorOwnerRef): string | null {
    if (owner.kind === "actor") {
      const synthetic = { system: { actor_id: owner.actor_id, overrides: {} } } as unknown as WireDocument;
      const eff = resolveTokenActor(synthetic, ctx.documents);
      return eff ? actorDisplayName(eff) : null;
    }
    const token = ctx.documents.get(owner.token_id);
    if (!token) return null;
    const eff = resolveTokenActor(token, ctx.documents);
    return eff ? actorDisplayName(eff) : null;
  }

  function formatTime(ms: number): string {
    const d = new Date(ms);
    return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
  }
  const timeLabel = $derived(formatTime(message.created_at));
  const timeTitle = $derived(new Date(message.created_at).toLocaleString());

  const whisperNames = $derived.by((): string => {
    if (sys?.audience.kind !== "whisper") return "";
    return sys.audience.recipients.map((r) => ctx.members.get(r) ?? r.slice(0, 8)).join(", ");
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
  <article class="card" class:emote={sys.kind === "emote"}>
    <header class="header">
      <span class="author">{authorName}</span>
      {#if actorName}
        <em class="actor-name">({actorName})</em>
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
        {#if sys.kind === "roll"}
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
                the ONLY string this app may ever pass to {@html}. -->
                <span class="seg-html">{@html s.sanitized_html}</span>
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
  // Single hook for emote styling — anchored on .card (the class:emote binding), not a
  // second class on the inner <p>, so there is one source of truth for "this is an emote".
  .card.emote .body p {
    font-style: italic;
  }
  .roll-pending {
    font-family: monospace;
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
    min-height: 32px;
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
</style>
