<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext, SegmentList } from "@shadowcat/ui-kit";
  import { resolveTokenBox, type WireDocument } from "@shadowcat/core";
  import {
    BUBBLE_MS,
    MAX_BUBBLES,
    addBubble,
    resolveSpeakerToken,
    scanNewMessages,
    seedSeen,
    tokenInViewedScope,
    type Bubble,
  } from "./bubbles";
  import type { ChatBubblesPreference } from "./preference.svelte";

  let { preference }: {
    /** This device's on/off toggle; while off no bubble is raised or drawn. */
    preference: ChatBubblesPreference;
  } = $props();

  const ctx = getAppContext();

  /** A live bubble resolved to a screen position. */
  interface PlacedBubble {
    /** The bubble to draw. */
    bubble: Bubble;
    /** Canvas-relative x of the token's top-centre, in pixels. */
    x: number;
    /** Canvas-relative y of the token's top edge, in pixels. */
    y: number;
  }

  let live = $state<Bubble[]>([]);
  /** Removal timers by message id, cleared on teardown. */
  const timers = new Map<string, ReturnType<typeof setTimeout>>();

  // `ctx.documents` is a plain store, not a rune: reading `subscribeDocuments()` first makes the
  // `placed` derivation re-run whenever the store emits (a token moved, a token was removed).
  const subscribeDocuments = createSubscriber((update) => ctx.documents.subscribe(update));

  // A bubble is drawn only while its token is on the viewed scene AND the fog the canvas paints
  // reveals the token's position (`stageView.isTokenRevealed`, fail-closed), so a bubble never
  // shows where a hidden token stands. Position tracks the token document's current x/y, so it
  // follows a token that moves while the bubble is up; it jumps to the destination rather than
  // following the movement tween, which the render layer owns.
  const placed = $derived.by((): PlacedBubble[] => {
    if (!preference.enabled) return [];
    subscribeDocuments();
    const footprints = ctx.footprints;
    const out: PlacedBubble[] = [];
    for (const bubble of live) {
      const token: WireDocument | undefined = ctx.documents.get(bubble.tokenId);
      if (!token || !tokenInViewedScope(bubble.tokenId, ctx.documents, ctx.viewedSceneId, ctx.viewedLevel)) continue;
      const box = resolveTokenBox(token, ctx.documents, footprints);
      if (!ctx.stageView.isTokenRevealed(bubble.tokenId, { x: box.x, y: box.y })) continue;
      const top = ctx.stageView.sceneToScreen({ x: box.x, y: box.y - box.h / 2 });
      out.push({ bubble, x: top.x, y: top.y });
    }
    return out;
  });

  /** Removes a bubble and its timer.
   * @param messageId The message id of the bubble to drop.
   * @example
   * ```
   * // private helper; not part of the public API
   * drop("m1");
   * ```
   */
  function drop(messageId: string): void {
    const timer = timers.get(messageId);
    if (timer !== undefined) clearTimeout(timer);
    timers.delete(messageId);
    live = live.filter((b) => b.messageId !== messageId);
  }

  /** Raises a bubble for each newly arrived eligible message whose speaker resolves to a token
   * on the viewed scene. Runs on every store emit; the seen-set bookkeeping inside
   * `scanNewMessages` runs even while the toggle is off, so switching it on never replays history.
   * @param seen The message ids already accounted for.
   * @example
   * ```
   * // private helper; not part of the public API
   * scan(new Set<string>());
   * ```
   */
  function scan(seen: Set<string>): void {
    for (const msg of scanNewMessages(seen, ctx.documents)) {
      if (!preference.enabled) continue;
      const tokenId = resolveSpeakerToken(msg.owner, ctx.documents, ctx.viewedSceneId, ctx.viewedLevel);
      if (!tokenId) continue;
      const replaced = live.find((b) => b.tokenId === tokenId);
      if (replaced) drop(replaced.messageId);
      live = addBubble(live, { messageId: msg.messageId, tokenId, channel: msg.channel, content: msg.content }, MAX_BUBBLES);
      timers.set(msg.messageId, setTimeout(() => drop(msg.messageId), BUBBLE_MS));
      // Bubbles evicted by the cap lose their timers too.
      for (const id of [...timers.keys()]) if (!live.some((b) => b.messageId === id)) drop(id);
    }
  }

  $effect(() => {
    const seen = new Set<string>();
    seedSeen(seen, ctx.documents);
    const off = ctx.documents.subscribe(() => scan(seen));
    return () => {
      off();
      for (const timer of timers.values()) clearTimeout(timer);
      timers.clear();
      live = [];
    };
  });
</script>

<div class="chat-bubbles" data-testid="chat-bubbles">
  {#each placed as p (p.bubble.messageId)}
    <div class="bubble" data-testid="chat-bubble" data-token-id={p.bubble.tokenId} style:left="{p.x}px" style:top="{p.y}px" aria-hidden="true">
      <SegmentList segments={p.bubble.content} channel={p.bubble.channel} />
    </div>
  {/each}
</div>

<style lang="scss">
  .chat-bubbles {
    position: absolute;
    inset: 0;
    overflow: hidden;
    pointer-events: none;
  }
  .bubble {
    position: absolute;
    // The anchor is the token's top edge; the bubble sits above it, centred, with a small gap.
    transform: translate(-50%, calc(-100% - var(--space-2)));
    max-width: min(16rem, 80%);
    max-height: 6lh;
    overflow: hidden;
    padding: var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-2);
    background: var(--surface-raised);
    color: var(--text-primary);
    box-shadow: var(--shadow-elevated);
    overflow-wrap: anywhere;
  }
</style>
