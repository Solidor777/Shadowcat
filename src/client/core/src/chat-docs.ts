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

/** Mirror of dice::outcome::DieRecord (M11d-2). Only the fields the roll card
 * renders are validated; `.passthrough()` tolerates server-only audit fields
 * (id, rerolled_from, ordered, ...) the client doesn't read, so an additive
 * server-side field never breaks this mirror. */
export const DieRecordSchema = z
  .object({
    value: z.number(),
    natural: z.number(),
    kept: z.boolean(),
    exploded: z.boolean(),
    crit_success: z.boolean(),
    crit_fail: z.boolean(),
    expertise: z.number(),
    group_index: z.number(),
    label: z.string().nullish(),
    symbols: z.array(z.string()),
  })
  .passthrough();
export type DieRecord = z.infer<typeof DieRecordSchema>;

/** Mirror of dice::outcome::RollOutcome (M11d-2). `successes`/`pass`/`margin`/
 * `tier_label`/`tier_value` are `None` in Total mode with no `difficulty`.
 * PRECISION: `total`/`margin` are i64 and — unlike wire.ts's seq/timestamp
 * fields — CAN legitimately reach i64::MAX/MIN (the evaluator saturates
 * overflowing constant/multiplication folds), beyond Number.MAX_SAFE_INTEGER;
 * JSON.parse rounds such extremes before Zod runs, so display precision
 * degrades past 2^53. Accepted tradeoff (no crash/security effect).
 * TODO: string-encode these two i64 fields if exact extreme totals matter. */
export const RollOutcomeSchema = z.object({
  total: z.number(),
  records: z.array(DieRecordSchema),
  successes: z.number().nullish(),
  pass: z.boolean().nullish(),
  margin: z.number().nullish(),
  tier_label: z.string().nullish(),
  tier_value: z.number().nullish(),
  crit_successes: z.number(),
  crit_fails: z.number(),
  positive_counter: z.number(),
  negative_counter: z.number(),
  symbol_counts: z.record(z.string(), z.number()),
});
export type RollOutcome = z.infer<typeof RollOutcomeSchema>;

/** Known segment kinds. `html.sanitized_html` is innerHTML-safe ONLY because the
 * server's chat::sanitize (ammonia) produced it — no client code may construct one.
 * `roll_embed.outcome` is a completed, immutable roll's full deterministic result
 * (chat/mod.rs Segment::RollEmbed); `roll_button` renders an unexecuted formula the
 * user can click to send a fresh `/roll` (chat/mod.rs Segment::RollButton).
 * `link_preview` mirrors chat/mod.rs Segment::LinkPreview — a server-fetched,
 * SSRF-guarded preview of a link in the message; the client renders ONLY the
 * stored `url`/`title`/`description` strings (escaped, never innerHTML) and never
 * fetches `url` itself (M11d-3). */
export const ChatSegmentSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("text"), text: z.string() }),
  z.object({ kind: z.literal("html"), sanitized_html: z.string() }),
  z.object({ kind: z.literal("roll_embed"), formula: z.string(), outcome: RollOutcomeSchema }),
  z.object({ kind: z.literal("roll_button"), formula: z.string(), label: z.string().nullish() }),
  z.object({ kind: z.literal("link_preview"), url: z.string(), title: z.string(), description: z.string() }),
]);
export type ChatSegment = z.infer<typeof ChatSegmentSchema>;
/** Forward-compat: a segment kind this client doesn't know (e.g. a future server's
 * DocLink) parses as opaque and renders as nothing — the message still shows.
 * INVARIANT: refuses every KNOWN kind — without this, a malformed
 * text/html/roll_embed/roll_button/link_preview segment (missing/wrong-typed
 * payload) would be rescued by this fallback and then misclassified as
 * trustworthy by isKnownSegment, breaking fail-closed. */
const UnknownSegmentSchema = z
  .object({ kind: z.string() })
  .passthrough()
  .refine(
    (s) =>
      s.kind !== "text" &&
      s.kind !== "html" &&
      s.kind !== "roll_embed" &&
      s.kind !== "roll_button" &&
      s.kind !== "link_preview",
  );
export type UnknownSegment = z.infer<typeof UnknownSegmentSchema>;
const SegmentListSchema = z.array(z.union([ChatSegmentSchema, UnknownSegmentSchema]));

export function isKnownSegment(s: ChatSegment | UnknownSegment): s is ChatSegment {
  return (
    s.kind === "text" ||
    s.kind === "html" ||
    s.kind === "roll_embed" ||
    s.kind === "roll_button" ||
    s.kind === "link_preview"
  );
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

/** Doc_type for the single per-world dice-settings config `Document`
 * (server: chat/settings.rs DICE_SETTINGS_DOC_TYPE). */
export const DICE_SETTINGS_DOC_TYPE = "dice-settings";

/** Mirror of chat::settings::DiceSettingsBody. Both fields serde-default on
 * the server (Total / high_wins), so a partial body is still safe there —
 * the panel always writes the full shape via the reactive seed. */
export interface DiceSettingsSystem {
  mode: "total" | "success_count";
  direction: "high_wins" | "low_wins";
}
export function buildDiceSettingsDoc(
  worldId: string,
  body: DiceSettingsSystem,
  id?: string,
): WireDocument {
  return envelope(worldId, DICE_SETTINGS_DOC_TYPE, null, body satisfies DiceSettingsSystem, id);
}

/** Doc_type for the single per-world chat-settings config `Document`
 * (server: chat/settings.rs CHAT_SETTINGS_DOC_TYPE). */
export const CHAT_SETTINGS_DOC_TYPE = "chat-settings";

/** Mirror of chat::settings::ChatContentPolicy. Every field `#[serde(default)]`
 * on the server (false), except `link_previews` which is tri-state: `undefined`
 * (absent) is the spec'd default-on-when-hyperlinks-on behavior
 * (`ChatContentPolicy::previews_enabled`), `true`/`false` are an explicit GM
 * override. A partial body is safe on the server; the panel writes single
 * fields via JSON-pointer update, never the whole doc. */
export interface ChatSettingsSystem {
  markdown?: boolean;
  html?: boolean;
  images?: boolean;
  hyperlinks?: boolean;
  emails?: boolean;
  link_previews?: boolean;
}
export function buildChatSettingsDoc(
  worldId: string,
  body: ChatSettingsSystem,
  id?: string,
): WireDocument {
  return envelope(worldId, CHAT_SETTINGS_DOC_TYPE, null, body satisfies ChatSettingsSystem, id);
}
