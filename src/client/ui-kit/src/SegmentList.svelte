<script lang="ts">
  import { getAppContext } from "./appContext";
  import type { ChatSegment, UnknownSegment, DocLinkTarget, WireActorOwnerRef } from "@shadowcat/core";
  import { isKnownSegment } from "@shadowcat/core";
  import RollTooltip from "./RollTooltip.svelte";

  /** A clickable "reroll this formula" segment, narrowed from `ChatSegment`. */
  type RollButtonSegment = Extract<ChatSegment, { /** Discriminant. */ kind: "roll_button" }>;

  let {
    segments,
    channel,
  }: {
    /** The message's raw segment list (including any forward-compat unknown entries). */
    segments: (ChatSegment | UnknownSegment)[];
    /** The carrying message's channel — a `roll_button` click sends its fresh `/roll` here. */
    channel: string;
  } = $props();

  const ctx = getAppContext();
  const t = ctx.t;

  /** Resolves a `doc_link` segment's target to an openable `SheetRef` when the referenced
   * document/token is present in the per-recipient OPTIMISTIC store: presence implies READ
   * (server-side redaction withholds an unauthorized doc from `base` entirely), and absence
   * renders inert plain text.
   * @param target The `doc_link` segment's target.
   * @returns The resolved open-ref, or `null` if the target isn't present in the store.
   * @example
   * ```
   * // internal; call sites use the derived per-segment resolution in the template
   * declare const target: DocLinkTarget;
   * docLinkOpenRef(target);
   * ```
   */
  function docLinkOpenRef(target: DocLinkTarget): { docId: string; embeddedPath?: string } | { tokenId: string } | null {
    if (target.kind === "doc") {
      return ctx.documents.get(target.doc_id)
        ? { docId: target.doc_id, embeddedPath: target.embedded_path ?? undefined }
        : null;
    }
    return ctx.documents.get(target.token_id) ? { tokenId: target.token_id } : null;
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

  /** The clickable href for a `link_preview`/`oembed` card, or `undefined` to render it
   * non-clickable. The server only ever stores an `http`/`https` URL for these segments; this
   * function does not trust that invariant across the wire boundary and independently
   * re-checks the scheme — a stored `javascript:`/`data:` URL must never become a live anchor.
   * @param url The stored URL.
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

  /** Roll-button click: a fresh, public, sender-attributed `/roll` on the carrying message's
   * channel — never re-executes the carrying message's own roll. A statted button is
   * per-clicker: the send carries the clicker's own actor binding (the pending one-shot
   * `speakAsToken` first, else the sticky session `speakAs`), so the server resolves a statted
   * template's references against the clicker's own actor, never the button author's stats.
   * @param s The clicked `roll_button` segment.
   * @example
   * ```
   * sendRollButton({ kind: "roll_button", formula: "1d20", label: null });
   * ```
   */
  function sendRollButton(s: RollButtonSegment): void {
    const pendingToken = ctx.speakAsToken.consume();
    const actorOwner: WireActorOwnerRef | undefined = pendingToken
      ? { kind: "token_instance", token_id: pendingToken }
      : ctx.speakAs.actorId
        ? { kind: "actor", actor_id: ctx.speakAs.actorId }
        : undefined;
    // The same precedence the composer applies (pending one-shot beats the sticky selection);
    // a server refusal (e.g. an unresolvable reference) is player-presentable, so surface it.
    void Promise.resolve(
      ctx.chat.send(actorOwner ? { channel, content: `/roll ${s.formula}`, actorOwner } : { channel, content: `/roll ${s.formula}` }),
    ).catch((e: unknown) => {
      ctx.notify(e instanceof Error ? e.message : String(e), "error");
    });
  }
</script>

{#each segments.filter(isKnownSegment) as s, i (i)}
  {#if s.kind === "text"}
    <span class="seg-text">{s.text}</span>
  {:else if s.kind === "html"}
    <!-- INVARIANT: sanitized_html is ammonia-cleaned by the server's chat::sanitize —
    the ONLY string this app may ever pass to {@html}. Every other segment kind
    (text, roll_embed, roll_button, link_preview, oembed, doc_link, image) renders via escaped
    interpolation only. link_preview/oembed thumbnails and image segments DO render an <img>,
    but its `src` is ALWAYS `ctx.assets.url(uuid, ...)` — this server's OWN /api/assets/{uuid}
    endpoint — never a raw external URL: the external fetch already happened server-side and
    the raw source URL is structurally never stored on any of those segments (only a Uuid asset
    id is), so there is no code path by which the viewer's browser could fetch a remote,
    attacker-chosen resource through this component. -->
    <span class="seg-html">{@html s.sanitized_html}</span>
  {:else if s.kind === "roll_embed"}
    <RollTooltip outcome={s.outcome} recalcHistory={s.recalc_history} />
  {:else if s.kind === "roll_button"}
    <button type="button" class="roll-btn" onclick={() => sendRollButton(s)}>
      {s.label ?? s.formula}
    </button>
  {:else if s.kind === "link_preview"}
    <!-- Server-fetched preview (SSRF-guarded). The client NEVER fetches
    `s.url` or any remote resource itself — title/description/url render as escaped
    text. `image_asset_id`, when present, is a post-publish-resolved thumbnail
    served through this server's OWN /api/assets/{uuid} endpoint (see the `html`
    branch's INVARIANT comment above); the raw external image URL is never stored
    and never reaches the client. -->
    <a class="link-preview" href={safeHref(s.url)} target="_blank" rel="noopener noreferrer nofollow">
      {#if s.image_asset_id}
        <img class="link-preview-thumb" src={ctx.assets.url(s.image_asset_id)} alt="" loading="lazy" />
      {/if}
      <span class="link-preview-title">{s.title}</span>
      <span class="link-preview-description">{s.description}</span>
      <span class="link-preview-host">{hostOf(s.url)}</span>
    </a>
  {:else if s.kind === "oembed"}
    <!-- Provider-native embed from an allowlisted host (see chat::oembed's module
    doc — no autodiscovery). STRUCTURED FIELDS ONLY: the provider's own `html` never
    reaches this client. The thumbnail, when present, is served the same way a
    link_preview's thumbnail is — through this server's OWN /api/assets/{uuid}
    endpoint, never hotlinked to the provider. -->
    <a class="oembed-card" href={safeHref(s.url)} target="_blank" rel="noopener noreferrer nofollow">
      {#if s.thumbnail_asset_id}
        <img class="oembed-thumb" src={ctx.assets.url(s.thumbnail_asset_id)} alt="" loading="lazy" />
      {/if}
      <span class="oembed-provider">{s.provider_name}</span>
      {#if s.title}<span class="oembed-title">{s.title}</span>{/if}
      {#if s.author_name}<span class="oembed-author">{s.author_name}</span>{/if}
      <span class="oembed-open">{t("chat.oembedOpenOn", { provider: s.provider_name })}</span>
    </a>
  {:else if s.kind === "doc_link"}
    {@const ref = docLinkOpenRef(s.target)}
    {#if ref}
      <button type="button" class="doc-link" onclick={() => ctx.openDocument(ref)}>{s.label}</button>
    {:else}
      <span class="seg-text">{s.label}</span>
    {/if}
  {:else if s.kind === "image"}
    <!-- Server-resolved asset: the `[[asset:...]]` span an author placed, or a
    Markdown/HTML image URL the server fetched and asset-ified. `src` is ALWAYS
    `ctx.assets.url(asset_id, ...)` — never a raw external URL (see the `html`
    branch's INVARIANT comment above). -->
    <a class="image-segment-link" href={ctx.assets.url(s.asset_id)} target="_blank" rel="noopener noreferrer">
      <img class="image-segment" src={ctx.assets.url(s.asset_id, "preview")} alt={s.alt} loading="lazy" />
    </a>
  {/if}
{/each}

<style lang="scss">
  .seg-text {
    // Preserves author-typed newlines in a plain-text segment; without this a multi-line
    // message collapses to one visual line despite the \n surviving in the DOM text node.
    white-space: pre-wrap;
  }
  .roll-btn {
    // Touch floor: matches the message card's `.actions` control sizing.
    min-height: 44px;
    min-width: 44px;
    padding: 0 var(--space-1);
  }
  // Images size to the card and sit on their own line, forced to a new line rather than
  // flowing inline with surrounding text. `max-width` caps the width; `display: block` forces
  // the break an inline <img> would not take.
  .seg-html :global(img) {
    max-width: 100%;
    display: block;
  }
  // Link-preview card: server-fetched title/description/host, all escaped text; an <img>
  // renders only when image_asset_id is present, its src always ctx.assets.url(uuid) — never
  // a raw external URL. The whole card is the link (44px touch floor on the anchor itself).
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
  .link-preview-thumb,
  .oembed-thumb {
    display: block;
    max-width: 100%;
    max-height: 160px;
    object-fit: cover;
    border-radius: var(--radius-1, 4px);
  }
  .oembed-card {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-1);
    border: 1px solid var(--border-color, #444);
    border-radius: var(--radius-1, 4px);
    text-decoration: none;
    color: inherit;
  }
  .oembed-provider {
    font-size: 0.85em;
    opacity: 0.7;
    text-transform: uppercase;
  }
  .oembed-open {
    font-size: 0.8em;
    opacity: 0.6;
  }
  .doc-link {
    background: none;
    border: none;
    padding: 0;
    color: var(--accent);
    cursor: pointer;
    font: inherit;
    text-decoration: underline;
  }
  .doc-link:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  // Image segment: sized like the html-sink's <img> and the preview thumbnails above, but
  // wrapped in its own anchor to the full-resolution asset (`ctx.assets.url(id)`, no variant)
  // rather than a link-preview's external click-through — clicking a chat image opens the
  // original, not a third-party page. 44px touch floor via min-height on the anchor, matching
  // `.link-preview`'s own floor.
  .image-segment-link {
    display: inline-block;
    min-height: 44px;
  }
  .image-segment {
    max-width: 100%;
    display: block;
    border-radius: var(--radius-1, 4px);
  }
</style>
