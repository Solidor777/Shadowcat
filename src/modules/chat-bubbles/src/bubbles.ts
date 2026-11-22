import {
  MESSAGE_DOC_TYPE,
  parseMessageEngine,
  type ChatSegment,
  type MessageKind,
  type ReadableDocuments,
  type UnknownSegment,
  type WireActorOwnerRef,
} from "@shadowcat/core";
import { sceneScopedDocs } from "@shadowcat/render";

/** How long a bubble stays on screen. Longer than a glyph emote because it carries readable
 * text. */
export const BUBBLE_MS = 8000;

/** Cap on bubbles alive at once; the oldest is evicted past it. The chat flood limit already
 * bounds message rate, so this only bounds the DOM node count if many tokens speak inside one
 * {@link BUBBLE_MS} window. */
export const MAX_BUBBLES = 8;

/** The message kinds that raise a bubble: spoken lines, emotes and rolls. Out-of-character and
 * system notices stay in the chat panel only. */
const BUBBLE_KINDS: ReadonlySet<MessageKind> = new Set<MessageKind>(["normal", "emote", "roll"]);

/** One live bubble: a message's content anchored to the token that spoke it. */
export interface Bubble {
  /** The message document id; unique per bubble. */
  messageId: string;
  /** The speaking token's document id. */
  tokenId: string;
  /** The message's channel, forwarded to `SegmentList` for a roll button's resend. */
  channel: string;
  /** The message's segment list, rendered through `SegmentList`. */
  content: (ChatSegment | UnknownSegment)[];
}

/** A message that has just arrived and may raise a bubble, before its speaker is resolved. */
export interface BubbleMessage {
  /** The message document id. */
  messageId: string;
  /** Who the message is spoken as. */
  owner: WireActorOwnerRef;
  /** The message's channel. */
  channel: string;
  /** The message's segment list. */
  content: (ChatSegment | UnknownSegment)[];
}

/**
 * Marks every message already in the store as seen, so the cold-start snapshot (history) never
 * raises a bubble. Call once, before the first {@link scanNewMessages}.
 * @param seen The set of message ids already accounted for (mutated in place).
 * @param documents The document view to seed from.
 * @example
 * ```ts
 * import { seedSeen } from "@shadowcat/module-chat-bubbles";
 * import type { ReadableDocuments } from "@shadowcat/core";
 *
 * declare const documents: ReadableDocuments;
 * seedSeen(new Set<string>(), documents);
 * ```
 */
export function seedSeen(seen: Set<string>, documents: ReadableDocuments): void {
  for (const doc of documents.query(MESSAGE_DOC_TYPE)) seen.add(doc.id);
}

/**
 * Collects the messages that arrived since the last scan and are eligible for a bubble: a
 * bubble kind, not deleted, spoken as an actor or token. Every message found is marked seen
 * whether or not it is eligible, so a later edit of the same message never raises a second bubble.
 * @param seen The set of message ids already accounted for (mutated in place).
 * @param documents The document view to scan.
 * @returns The eligible new messages, in store order.
 * @example
 * ```ts
 * import { scanNewMessages } from "@shadowcat/module-chat-bubbles";
 * import type { ReadableDocuments } from "@shadowcat/core";
 *
 * declare const documents: ReadableDocuments;
 * scanNewMessages(new Set<string>(), documents);
 * ```
 */
export function scanNewMessages(seen: Set<string>, documents: ReadableDocuments): BubbleMessage[] {
  const out: BubbleMessage[] = [];
  for (const doc of documents.query(MESSAGE_DOC_TYPE)) {
    if (seen.has(doc.id)) continue;
    seen.add(doc.id);
    const engine = parseMessageEngine(doc);
    if (!engine || !BUBBLE_KINDS.has(engine.kind) || engine.deleted_at != null || !engine.actor_owner) continue;
    out.push({ messageId: doc.id, owner: engine.actor_owner, channel: engine.channel, content: engine.content });
  }
  return out;
}

/**
 * Whether a token is placed on the viewed scene and level, by the scope the stage renders
 * (`sceneScopedDocs`).
 * @param tokenId The token's document id.
 * @param documents The document view to read tokens from.
 * @param viewedSceneId The scene the client renders.
 * @param viewedLevel The level the client renders, or `null`.
 * @returns `true` when the token is in scope.
 * @example
 * ```ts
 * import { tokenInViewedScope } from "@shadowcat/module-chat-bubbles";
 * import type { ReadableDocuments } from "@shadowcat/core";
 *
 * declare const documents: ReadableDocuments;
 * tokenInViewedScope("t1", documents, "s1", null);
 * ```
 */
export function tokenInViewedScope(
  tokenId: string,
  documents: ReadableDocuments,
  viewedSceneId: string | null,
  viewedLevel: string | null,
): boolean {
  return sceneScopedDocs(documents, "token", () => viewedSceneId, () => viewedLevel).some((tok) => tok.id === tokenId);
}

/**
 * Resolves a message's speaker to a token placed on the viewed scene and level, using the same
 * scene scope the stage renders (`sceneScopedDocs`). A `token_instance` owner names its token
 * directly; an `actor` owner resolves only when exactly one in-scope token is linked to that
 * actor, since several would make the speaker ambiguous.
 * @param owner The message's actor attribution.
 * @param documents The document view to read tokens from.
 * @param viewedSceneId The scene the client renders.
 * @param viewedLevel The level the client renders, or `null`.
 * @returns The speaking token's id, or `null` when it is not on the viewed scene or is ambiguous.
 * @example
 * ```ts
 * import { resolveSpeakerToken } from "@shadowcat/module-chat-bubbles";
 * import type { ReadableDocuments } from "@shadowcat/core";
 *
 * declare const documents: ReadableDocuments;
 * resolveSpeakerToken({ kind: "token_instance", token_id: "t1" }, documents, "s1", null);
 * ```
 */
export function resolveSpeakerToken(
  owner: WireActorOwnerRef,
  documents: ReadableDocuments,
  viewedSceneId: string | null,
  viewedLevel: string | null,
): string | null {
  if (owner.kind === "token_instance") {
    return tokenInViewedScope(owner.token_id, documents, viewedSceneId, viewedLevel) ? owner.token_id : null;
  }
  const tokens = sceneScopedDocs(documents, "token", () => viewedSceneId, () => viewedLevel);
  const linked = tokens.filter((tok) => (tok.engine as { /** The linked actor id, if any. */ actor_id?: string | null }).actor_id === owner.actor_id);
  return linked.length === 1 ? linked[0].id : null;
}

/**
 * Adds a bubble to the live list. A token shows one bubble at a time, so a newer bubble replaces
 * that token's older one; past `max` the oldest bubble is evicted.
 * @param live The live bubbles, oldest first.
 * @param bubble The bubble to add.
 * @param max The most bubbles to keep.
 * @returns The new live list, oldest first.
 * @example
 * ```ts
 * import { addBubble, MAX_BUBBLES } from "@shadowcat/module-chat-bubbles";
 *
 * addBubble([], { messageId: "m1", tokenId: "t1", channel: "general", content: [] }, MAX_BUBBLES);
 * ```
 */
export function addBubble(live: readonly Bubble[], bubble: Bubble, max: number): Bubble[] {
  return [...live.filter((b) => b.tokenId !== bubble.tokenId), bubble].slice(-max);
}
