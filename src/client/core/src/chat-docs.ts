// Client mirror of the chat message content model (src/server/src/chat/mod.rs).
// The body types are serde-only on the server (NO ts-rs) — this file is the
// manually-kept-in-sync Zod mirror; a Rust body-shape change MUST update it.
// Fail-closed: a body that does not parse renders as nothing, never partially.
import { z } from "zod";
import { ActorOwnerRefSchema, AudienceSchema, type WireDocument } from "./wire";
import { envelope } from "./scene-docs";

export const MESSAGE_DOC_TYPE = "message";
export const CHANNEL_REGISTRY_DOC_TYPE = "channel-registry";
/** Server-enforced content cap (chat/mod.rs MAX_MESSAGE_CHARS) — composer pre-validates. */
export const MAX_MESSAGE_CHARS = 4096;

export const MessageKindSchema = z.enum(["normal", "emote", "roll", "system"]);
export type MessageKind = z.infer<typeof MessageKindSchema>;

/** Known segment kinds. `html.sanitized_html` is innerHTML-safe ONLY because the
 * server's chat::sanitize (ammonia) produced it — no client code may construct one. */
export const ChatSegmentSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("text"), text: z.string() }),
  z.object({ kind: z.literal("html"), sanitized_html: z.string() }),
]);
export type ChatSegment = z.infer<typeof ChatSegmentSchema>;
/** Forward-compat: a segment kind this client doesn't know (e.g. a newer server's
 * roll_embed) parses as opaque and renders as nothing — the message still shows.
 * INVARIANT: refuses the KNOWN kinds — without this, a malformed text/html
 * segment (missing/wrong-typed payload) would be rescued by this fallback and
 * then misclassified as trustworthy by isKnownSegment, breaking fail-closed. */
const UnknownSegmentSchema = z
  .object({ kind: z.string() })
  .passthrough()
  .refine((s) => s.kind !== "text" && s.kind !== "html");
export type UnknownSegment = z.infer<typeof UnknownSegmentSchema>;
const SegmentListSchema = z.array(z.union([ChatSegmentSchema, UnknownSegmentSchema]));

export function isKnownSegment(s: ChatSegment | UnknownSegment): s is ChatSegment {
  return s.kind === "text" || s.kind === "html";
}

export const ChatMessageSystemSchema = z.object({
  channel: z.string(),
  user_owner: z.string(),
  actor_owner: ActorOwnerRefSchema.nullish(),
  kind: MessageKindSchema,
  audience: AudienceSchema.default({ kind: "public" }),
  content: SegmentListSchema,
  source: z.string().nullish(),
  edited_at: z.number().nullish(),
  deleted_at: z.number().nullish(),
});
export type ChatMessageSystem = z.infer<typeof ChatMessageSystemSchema>;

/** Fail-closed body parse: null unless `doc` is a message with a valid body. */
export function parseMessageSystem(doc: WireDocument): ChatMessageSystem | null {
  if (doc.doc_type !== MESSAGE_DOC_TYPE) return null;
  const r = ChatMessageSystemSchema.safeParse(doc.system);
  return r.success ? r.data : null;
}

/** A chat channel's display config. Channels are a purely client-side label
 * taxonomy — the server never validates `channel` (chat skill: audience, not
 * channel, is the only server-enforced visibility). */
export interface ChatChannel {
  name: string;
}
/** Singleton config doc (doc_type "channel-registry"): id→channel MAP, so
 * add/rename/remove are single-key field Updates (set_pointer cannot grow arrays). */
export interface ChannelRegistrySystem {
  channels: Record<string, ChatChannel>;
}
export function buildChannelRegistryDoc(
  worldId: string,
  channels: Record<string, ChatChannel>,
  id?: string,
): WireDocument {
  return envelope(worldId, CHANNEL_REGISTRY_DOC_TYPE, null, { channels } satisfies ChannelRegistrySystem, id);
}
