// Runtime validation for the WebSocket wire protocol. The compile-time wire
// types come from `@shadowcat/types` (ts-rs output); these Zod schemas validate
// inbound server frames at the trust boundary. `wire.test.ts` guards them
// against drift from the Rust types.
//
// i64/u32 fields arrive as JSON numbers and are modeled as `number` (seq and
// millisecond timestamps stay well within 2^53). ts-rs types i64 as `bigint`;
// using `number` keeps JSON.parse/stringify ergonomic (bigint is not
// JSON-serializable). The drift guard normalizes that one scalar difference.
import { z } from "zod";

/** A wire integer (i64/u32) — see the module note on number vs bigint. */
const int = z.number().int();

export const DocRoleSchema = z.enum(["owner", "observer", "none"]);
export const VisibilitySchema = z.enum(["all", "gm_only"]);
export const WorldRoleSchema = z.enum(["gm", "player", "spectator"]);
export const RejectReasonSchema = z.enum(["forbidden", "conflict", "invalid"]);
export const ResyncSourceSchema = z.enum(["buffer", "log"]);
export const WsErrorCodeSchema = z.enum([
  "world_not_found",
  "bad_message",
  "publish_failed",
  "forbidden",
  "internal",
]);

export const ScopeSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("compendium"), pack: z.string() }),
  z.object({ kind: z.literal("world"), world_id: z.string() }),
]);

export const SourceSchema = z.object({
  id: z.string(),
  pack: z.string().nullable(),
  version: int,
});

export const CapabilityGrantsSchema = z.object({
  by_role: z.record(z.array(z.string())),
  by_user: z.record(z.array(z.string())),
});

export const PermissionSetSchema = z.object({
  default: DocRoleSchema,
  users: z.record(DocRoleSchema),
  property_overrides: z.record(VisibilitySchema),
  capabilities: CapabilityGrantsSchema,
});

/** The validated document shape (`bigint` i64 fields modeled as `number`). */
export type WireDocument = {
  id: string;
  scope: z.infer<typeof ScopeSchema>;
  doc_type: string;
  schema_version: number;
  source: z.infer<typeof SourceSchema> | null;
  owner: string | null;
  permissions: z.infer<typeof PermissionSetSchema>;
  embedded: Record<string, WireDocument[]>;
  // `z.unknown()` infers an optional property; the value is the opaque system body.
  system?: unknown;
  created_at: number;
  updated_at: number;
};

// `embedded` holds child documents, so the schema is recursive (z.lazy).
export const DocumentSchema: z.ZodType<WireDocument> = z.lazy(() =>
  z.object({
    id: z.string(),
    scope: ScopeSchema,
    doc_type: z.string(),
    schema_version: int,
    source: SourceSchema.nullable(),
    owner: z.string().nullable(),
    permissions: PermissionSetSchema,
    embedded: z.record(z.array(DocumentSchema)),
    system: z.unknown(),
    created_at: int,
    updated_at: int,
  }),
);

export const FieldChangeSchema = z.object({
  path: z.string(),
  old: z.unknown(),
  new: z.unknown(),
});

export const OperationSchema = z.discriminatedUnion("op", [
  z.object({ op: z.literal("create"), doc: DocumentSchema }),
  z.object({ op: z.literal("delete"), doc: DocumentSchema }),
  z.object({
    op: z.literal("update"),
    doc_id: z.string(),
    changes: z.array(FieldChangeSchema),
  }),
]);

export const CommandSchema = z.object({
  seq: int,
  world_id: z.string(),
  author: z.string(),
  ts: int,
  ops: z.array(OperationSchema),
});

export const ServerMsgSchema = z.discriminatedUnion("type", [
  z.object({
    type: z.literal("welcome"),
    world: z.string(),
    current_seq: int,
    server_time: int,
  }),
  z.object({
    type: z.literal("event"),
    command: CommandSchema,
    intent_id: z.string().nullable(),
  }),
  z.object({
    type: z.literal("reject"),
    intent_id: z.string(),
    reason: RejectReasonSchema,
  }),
  z.object({
    type: z.literal("resync_begin"),
    from_seq: int,
    to_seq: int,
    source: ResyncSourceSchema,
  }),
  z.object({ type: z.literal("resync_end"), current_seq: int }),
  z.object({
    type: z.literal("time_pong"),
    client_t0: int,
    server_t: int,
  }),
  z.object({ type: z.literal("ping") }),
  z.object({
    type: z.literal("error"),
    code: WsErrorCodeSchema,
    message: z.string(),
  }),
]);

export type WireScope = z.infer<typeof ScopeSchema>;
export type WireFieldChange = z.infer<typeof FieldChangeSchema>;
export type WireOperation = z.infer<typeof OperationSchema>;
export type WireCommand = z.infer<typeof CommandSchema>;
export type ServerMsg = z.infer<typeof ServerMsgSchema>;

/** Client -> server frames. Plain objects (numbers, JSON.stringify-friendly). */
export type ClientMsg =
  | { type: "hello"; world: string; last_seq: number | null }
  | { type: "intent"; intent_id: string; ops: WireOperation[] }
  | { type: "resync_request"; from_seq: number }
  | { type: "time_ping"; client_t0: number }
  | { type: "pong" };

/** Parse + validate an inbound text frame; `null` on malformed/unknown input. */
export function parseServerMsg(text: string): ServerMsg | null {
  const json = ((): unknown => {
    try {
      return JSON.parse(text);
    } catch {
      return undefined;
    }
  })();
  const result = ServerMsgSchema.safeParse(json);
  return result.success ? result.data : null;
}
