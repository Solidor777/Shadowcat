// Client mirror of the chat message content model (`chat::MessageEngine` et al).
// `MessageEngine` is deliberately NOT ts-rs-exported (server comment: "Opaque on the
// WIRE" — the server enforces `deny_unknown_fields` structurally, the exact segment/
// outcome union is the client's own concern) — this file is the manually-kept-in-sync
// Zod mirror; a Rust body-shape change MUST update it. Fail-closed: a body that does
// not parse renders as nothing, never partially. Re-rooted onto the three-band
// document shape: the message body lives in `doc.engine`, `doc.system` stays `{}`.
import { z } from "zod";
import { ActorOwnerRefSchema, AudienceSchema, type WireDocument } from "./wire";
import { envelope } from "./scene-docs";
import type { ChannelRegistryEngine, ChatSettingsEngine, DiceSettingsEngine } from "@shadowcat/types";
export type { ChannelRegistryEngine, ChatSettingsEngine, DiceSettingsEngine };

export const MESSAGE_DOC_TYPE = "message";
export const CHANNEL_REGISTRY_DOC_TYPE = "channel-registry";
/** Server-enforced content cap (`chat::MAX_MESSAGE_CHARS`) — composer pre-validates. */
export const MAX_MESSAGE_CHARS = 4096;

export const MessageKindSchema = z.enum(["normal", "emote", "roll", "system"]);
/** The inferred TS shape of `MessageKindSchema`. */
export type MessageKind = z.infer<typeof MessageKindSchema>;

/** Mirror of `dice::outcome::DieRecord`. Only the fields the roll card
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
/** The inferred TS shape of `DieRecordSchema`. */
export type DieRecord = z.infer<typeof DieRecordSchema>;

/** Mirror of dice::spec::ConstTerm — a labeled bare constant. Display/
 * provenance decoration only (never fed into a dice-pool comparison); only
 * ever populated in Total mode. */
export const ConstTermSchema = z.object({
  value: z.number(),
  label: z.string().nullish(),
});
/** The inferred TS shape of `ConstTermSchema`. */
export type ConstTerm = z.infer<typeof ConstTermSchema>;

/** Mirror of `dice::outcome::RollOutcome`. `successes`/`pass`/`margin`/
 * `tier_label`/`tier_value` are `None` in Total mode with no `difficulty`.
 * PRECISION: `total`/`margin` are i64 and — unlike this package's other wire-protocol
 * seq/timestamp fields — CAN legitimately reach i64::MAX/MIN (the evaluator saturates
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
  /** Every labeled `Const` term in the expression; empty in SuccessCount mode
   * (arithmetic is ignored there) and defaults to empty for any roll
   * persisted before this field existed. */
  labeled_consts: z.array(ConstTermSchema).default([]),
});
/** The inferred TS shape of `RollOutcomeSchema`. */
export type RollOutcome = z.infer<typeof RollOutcomeSchema>;

/** Known segment kinds. `html.sanitized_html` is innerHTML-safe ONLY because the
 * server's `chat::sanitize` (ammonia) produced it — no client code may construct one.
 * `roll_embed.outcome` is a completed, immutable roll's full deterministic result
 * (`chat::Segment::RollEmbed`); `roll_button` renders an unexecuted formula the
 * user can click to send a fresh `/roll` (`chat::Segment::RollButton`).
 * `link_preview` mirrors `chat::Segment::LinkPreview` — a server-fetched,
 * SSRF-guarded preview of a link in the message; the client renders ONLY the
 * stored `url`/`title`/`description` strings (escaped, never innerHTML) and never
 * fetches `url` itself. */
export const ChatSegmentSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("text"), text: z.string() }),
  z.object({ kind: z.literal("html"), sanitized_html: z.string() }),
  z.object({ kind: z.literal("roll_embed"), formula: z.string(), outcome: RollOutcomeSchema }),
  z.object({ kind: z.literal("roll_button"), formula: z.string(), label: z.string().nullish() }),
  z.object({ kind: z.literal("link_preview"), url: z.string(), title: z.string(), description: z.string() }),
]);
/** The inferred TS shape of `ChatSegmentSchema` — one of the five known segment kinds. */
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
/** The inferred TS shape of `UnknownSegmentSchema` — a forward-compat, not-yet-known segment kind. */
export type UnknownSegment = z.infer<typeof UnknownSegmentSchema>;
const SegmentListSchema = z.array(z.union([ChatSegmentSchema, UnknownSegmentSchema]));

/** Narrows a parsed segment to a known `ChatSegment` kind. See the type guard's
 * companion `UnknownSegmentSchema` note: this fallback deliberately refuses
 * every known `kind` string, so a malformed known-kind segment fails the
 * whole message rather than being misclassified as trustworthy here.
 * @param s The parsed segment (known or opaque forward-compat).
 * @returns `true` if `s.kind` is one of `text`/`html`/`roll_embed`/`roll_button`/`link_preview`.
 * @example
 * ```ts
 * import { isKnownSegment } from "@shadowcat/core";
 *
 * isKnownSegment({ kind: "text", text: "hello" });
 * ```
 */
export function isKnownSegment(s: ChatSegment | UnknownSegment): s is ChatSegment {
  return (
    s.kind === "text" ||
    s.kind === "html" ||
    s.kind === "roll_embed" ||
    s.kind === "roll_button" ||
    s.kind === "link_preview"
  );
}

export const ChatMessageEngineSchema = z.object({
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
/** The inferred TS shape of `ChatMessageEngineSchema` — a message document's `engine` body. */
export type ChatMessageEngine = z.infer<typeof ChatMessageEngineSchema>;

/** Fail-closed body parse: null unless `doc` is a message with a valid `engine` body.
 * @param doc The candidate document.
 * @returns The parsed `ChatMessageEngine`, or `null` for a non-message `doc_type` or a malformed body.
 * @example
 * ```ts
 * import { parseMessageEngine } from "@shadowcat/core";
 * import type { WireDocument } from "@shadowcat/core";
 *
 * declare const doc: WireDocument;
 * parseMessageEngine(doc);
 * ```
 */
export function parseMessageEngine(doc: WireDocument): ChatMessageEngine | null {
  if (doc.doc_type !== MESSAGE_DOC_TYPE) return null;
  const r = ChatMessageEngineSchema.safeParse(doc.engine);
  return r.success ? r.data : null;
}

/** Singleton config doc (doc_type "channel-registry"): id→channel MAP, so
 * add/rename/remove are single-key field Updates (set_pointer cannot grow arrays).
 * `doc_type: "channel-registry"` is engine-defined — the map lands in `engine`.
 * @param worldId The owning world's id.
 * @param channels The channel-id → `Channel` map.
 * @param id Optional explicit document id; a fresh uuid is generated when omitted.
 * @returns The unsaved `WireDocument`, ready to `Create`.
 * @example
 * ```ts
 * import { buildChannelRegistryDoc } from "@shadowcat/core";
 *
 * buildChannelRegistryDoc("00000000-0000-0000-0000-000000000001", { general: { name: "General" } });
 * ```
 */
export function buildChannelRegistryDoc(
  worldId: string,
  channels: ChannelRegistryEngine["channels"],
  id?: string,
): WireDocument {
  return envelope(worldId, CHANNEL_REGISTRY_DOC_TYPE, null, {}, id, { channels } satisfies ChannelRegistryEngine, null);
}

/** Doc_type for the single per-world dice-settings config `Document`
 * (server: `chat::settings::DICE_SETTINGS_DOC_TYPE`). `doc_type: "dice-settings"` is
 * engine-defined — the body lands in `engine`, `DiceSettingsEngine` mirrors
 * data::engine::registries::DiceSettingsEngine 1:1 (both fields serde-default on the server:
 * Total / high_wins), so a partial body is still safe — the panel always writes the
 * full shape via the reactive seed. */
export const DICE_SETTINGS_DOC_TYPE = "dice-settings";

/** Builds the singleton per-world `dice-settings` config document.
 * @param worldId The owning world's id.
 * @param engine The dice-settings body (mode + win direction).
 * @param id Optional explicit document id; a fresh uuid is generated when omitted.
 * @returns The unsaved `WireDocument`, ready to `Create`.
 * @example
 * ```ts
 * import { buildDiceSettingsDoc } from "@shadowcat/core";
 *
 * buildDiceSettingsDoc("00000000-0000-0000-0000-000000000001", { mode: "total", direction: "high_wins" });
 * ```
 */
export function buildDiceSettingsDoc(
  worldId: string,
  engine: DiceSettingsEngine,
  id?: string,
): WireDocument {
  return envelope(worldId, DICE_SETTINGS_DOC_TYPE, null, {}, id, engine satisfies DiceSettingsEngine, null);
}

/** Doc_type for the single per-world chat-settings config `Document`
 * (server: `chat::settings::CHAT_SETTINGS_DOC_TYPE`). `doc_type: "chat-settings"` is
 * engine-defined — the body lands in `engine`, `ChatSettingsEngine` mirrors
 * chat::settings::ChatContentPolicy: every field `#[serde(default)]` on the server
 * (false), except `link_previews` which is tri-state: absent/`null` defaults previews ON
 * whenever `hyperlinks` is also on (`ChatContentPolicy::previews_enabled`), and OFF when
 * `hyperlinks` is off regardless of this field; `true`/`false` is an explicit GM override.
 * The panel writes single fields via
 * JSON-pointer update, never the whole doc. */
export const CHAT_SETTINGS_DOC_TYPE = "chat-settings";

/** Builds the singleton per-world `chat-settings` config document.
 * @param worldId The owning world's id.
 * @param engine The chat content-policy body (markdown/html/images/hyperlinks/emails/link_previews toggles).
 * @param id Optional explicit document id; a fresh uuid is generated when omitted.
 * @returns The unsaved `WireDocument`, ready to `Create`.
 * @example
 * ```ts
 * import { buildChatSettingsDoc } from "@shadowcat/core";
 *
 * buildChatSettingsDoc("00000000-0000-0000-0000-000000000001", {
 *   markdown: true,
 *   html: null,
 *   images: null,
 *   hyperlinks: true,
 *   emails: null,
 *   link_previews: null,
 * });
 * ```
 */
export function buildChatSettingsDoc(
  worldId: string,
  engine: ChatSettingsEngine,
  id?: string,
): WireDocument {
  return envelope(worldId, CHAT_SETTINGS_DOC_TYPE, null, {}, id, engine satisfies ChatSettingsEngine, null);
}
