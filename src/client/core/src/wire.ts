// Runtime validation for the WebSocket wire protocol. The compile-time wire
// types come from `@shadowcat/types` (ts-rs output); these Zod schemas validate
// inbound server frames at the trust boundary, plus outbound client-frame
// schemas (e.g. `SendMessageSchema`) that callers may opt to validate before
// sending. `wire.test.ts` guards them against drift from the Rust types.
//
// i64/u32 fields arrive as JSON numbers and are modeled as `number` (seq and
// millisecond timestamps stay well within 2^53). ts-rs types i64 as `bigint`;
// using `number` keeps JSON.parse/stringify ergonomic (bigint is not
// JSON-serializable). The drift guard normalizes that one scalar difference.
import { z } from "zod";

/** A wire integer (i64/u32) — see the module note on number vs bigint. */
const int = z.number().int();

export const DocRoleSchema = z.enum(["owner", "observer", "none"]);
export const VisibilitySchema = z.enum(["all", "gm_only", "owner_or_gm"]);
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

/** Mirrors `crate::chat::ActorOwnerRef` (chat message attribution). */
export const ActorOwnerRefSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("actor"), actor_id: z.string() }),
  z.object({ kind: z.literal("token_instance"), token_id: z.string() }),
]);
export type WireActorOwnerRef = z.infer<typeof ActorOwnerRefSchema>;

/** Mirrors `crate::chat::Audience` (chat message readership). */
export const AudienceSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("public") }),
  z.object({ kind: z.literal("whisper"), recipients: z.array(z.string()) }),
  z.object({ kind: z.literal("gm_only") }),
]);
export type WireAudience = z.infer<typeof AudienceSchema>;

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

export const CapabilityRequirementSchema = z.object({
  path_prefix: z.string(),
  caps: z.array(z.string()),
});
export type WireCapabilityRequirement = z.infer<typeof CapabilityRequirementSchema>;

export const CardinalitySchema = z.enum(["singleton", "multi"]);

export const ContractProvideSchema = z.object({
  contract: z.string(),
  cardinality: CardinalitySchema,
});

export const ContractDeclarationSchema = z.object({
  module_id: z.string(),
  version: z.string(),
  provides: z.array(ContractProvideSchema),
  requires: z.array(z.string()),
});
export type WireContractDeclaration = z.infer<typeof ContractDeclarationSchema>;

export const SchemaTypeSchema = z.enum([
  "object",
  "array",
  "string",
  "number",
  "boolean",
  "null",
]);

// Recursive structural type-tree (M13f tier-2). `additionalProperties` is
// `boolean | Schema`; absent fields are optional (server omits None via
// skip_serializing_if). Shape only — never a value rule.
export type WireSchema = {
  type?: z.infer<typeof SchemaTypeSchema>;
  properties?: Record<string, WireSchema>;
  required?: string[];
  additionalProperties?: boolean | WireSchema;
  items?: WireSchema;
  nullable?: boolean;
};

export const SchemaSchema: z.ZodType<WireSchema> = z.lazy(() =>
  z.object({
    type: SchemaTypeSchema.optional(),
    properties: z.record(SchemaSchema).optional(),
    required: z.array(z.string()).optional(),
    additionalProperties: z.union([z.boolean(), SchemaSchema]).optional(),
    items: SchemaSchema.optional(),
    nullable: z.boolean().optional(),
  }),
);

export const SchemaDeclarationSchema = z.object({
  module_id: z.string(),
  version: z.string(),
  schema_format: int,
  doc_type: z.string(),
  subtree_pointer: z.string(),
  schema: SchemaSchema,
});
export type WireSchemaDeclaration = z.infer<typeof SchemaDeclarationSchema>;

export const PermissionSetSchema = z.object({
  default: DocRoleSchema,
  users: z.record(DocRoleSchema),
  property_overrides: z.record(VisibilitySchema),
  capabilities: CapabilityGrantsSchema,
  gm_role: DocRoleSchema.nullable(),
});

/** The validated document shape (`bigint` i64 fields modeled as `number`). */
export type WireDocument = {
  id: string;
  scope: z.infer<typeof ScopeSchema>;
  doc_type: string;
  schema_version: number;
  // Universal display name. Redacts to `null` under a `/name` override.
  name: string | null;
  source: z.infer<typeof SourceSchema> | null;
  // Opaque mergeable-content snapshot at last sync (`MergeBase`, `./merge`). Present only on
  // stamped children; absent/undefined otherwise. Server-opaque; the client owns the shape.
  base?: unknown;
  owner: string | null;
  permissions: z.infer<typeof PermissionSetSchema>;
  embedded: Record<string, WireDocument[]>;
  // Scene-entity link: the parent scene's id (or other parent); null for top-level
  // docs (actors, compendium entries, scenes). Immutable via field-path Update.
  parent_id: string | null;
  // Engine band: present iff `doc_type` is engine-defined; validated + typed via the
  // generated `*Engine` structs (`@shadowcat/types`). `z.unknown()` infers an optional
  // property, so an absent/non-engine doc_type's `engine` key is simply undefined.
  engine?: unknown;
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
    name: z.string().nullable(),
    source: SourceSchema.nullable(),
    base: z.unknown(),
    owner: z.string().nullable(),
    permissions: PermissionSetSchema,
    embedded: z.record(z.array(DocumentSchema)),
    parent_id: z.string().nullable(),
    engine: z.unknown(),
    system: z.unknown(),
    created_at: int,
    updated_at: int,
  }),
);

export const FieldChangeSchema = z.object({
  path: z.string(),
  old: z.unknown(),
  new: z.unknown(),
  // When true, REMOVE the object key at `path` (genuine absence) instead of
  // setting `new`. Omitted on the wire when false (mirrors the server's
  // `#[serde(skip_serializing_if)]`).
  remove: z.boolean().optional(),
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

export const SearchHitSchema = z.object({
  document: DocumentSchema,
  score: z.number(),
  snippet: z.string(),
});
export type WireSearchHit = z.infer<typeof SearchHitSchema>;

export const ServerMsgSchema = z.discriminatedUnion("type", [
  z.object({
    type: z.literal("welcome"),
    world: z.string(),
    current_seq: int,
    server_time: int,
    server_version: z.string(),
    world_default_grants: CapabilityGrantsSchema,
    user_role: WorldRoleSchema,
    capability_requirements: z.array(CapabilityRequirementSchema),
    contract_declarations: z.array(ContractDeclarationSchema),
    schema_declarations: z.array(SchemaDeclarationSchema),
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
  z.object({
    type: z.literal("search_result"),
    request_id: z.string(),
    hits: z.array(SearchHitSchema),
    next_cursor: z.string().nullable(),
  }),
  z.object({
    type: z.literal("search_error"),
    request_id: z.string(),
    message: z.string(),
  }),
  z.object({
    type: z.literal("search_update"),
    request_id: z.string(),
    hits: z.array(SearchHitSchema),
  }),
  z.object({
    type: z.literal("scene_derived"),
    request_id: z.string(),
    channel: z.string(),
    computed_at_seq: int,
    payload: z.unknown(),
  }),
  z.object({
    type: z.literal("scene_error"),
    request_id: z.string(),
    message: z.string(),
  }),
  z.object({
    type: z.literal("asset_changed"),
    uuid: z.string(),
    op: z.enum(["replaced", "deleted"]),
  }),
  z.object({
    type: z.literal("scene_ping"),
    scene: z.string(),
    x: z.number(),
    y: z.number(),
    user: z.string(),
  }),
  z.object({
    type: z.literal("path_result"),
    request_id: z.string(),
    path: z.array(z.tuple([z.number(), z.number()])),
    cost: z.number(),
    arrested: z.boolean(),
  }),
  z.object({
    type: z.literal("path_error"),
    request_id: z.string(),
    message: z.string(),
  }),
  z.object({
    type: z.literal("move_error"),
    request_id: z.string(),
    message: z.string(),
  }),
  z.object({
    type: z.literal("chat_error"),
    request_id: z.string(),
    message: z.string(),
  }),
  z.object({
    type: z.literal("move_stream"),
    request_id: z.string(),
    token_id: z.string(),
    mover: z.string(),
    scene: z.string(),
    start_server_ms: z.number(),
    duration_ms: z.number(),
    stop: z.tuple([z.number(), z.number()]),
    samples: z.array(
      z.object({
        t_ms: z.number(),
        pos: z.tuple([z.number(), z.number()]),
      }),
    ),
    mover_vision: z
      .array(
        z.object({
          t_ms: z.number(),
          polygons: z.array(z.array(z.tuple([z.number(), z.number()]))),
        }),
      )
      .nullable(),
    // Null for a clipped observer (mirrors mover_vision) — the authoritative cost may
    // reflect secret-region terrain the observer's clipped samples don't reveal.
    cost: z.number().nullable(),
  }),
  // Terminal eviction (world or account deletion); the server closes the
  // socket right after. Terminal: the client must stop, not reconnect.
  z.object({ type: z.literal("evicted"), user: z.string().nullable() }),
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
  | { type: "pong" }
  | {
      type: "search";
      request_id: string;
      query: string;
      limit: number;
      cursor?: string;
      subscribe: boolean;
    }
  | { type: "unsubscribe"; request_id: string }
  | { type: "scene_subscribe"; request_id: string; channel: string }
  | { type: "scene_unsubscribe"; request_id: string }
  | { type: "scene_ping"; scene: string; x: number; y: number }
  | {
      type: "pathfind";
      request_id: string;
      scene: string;
      start: [number, number];
      waypoints: [number, number][];
      footprint_radius: number;
      token?: string;
    }
  | {
      type: "move_request";
      request_id: string;
      scene: string;
      token_id: string;
      path: [number, number][];
    }
  | {
      type: "send_message";
      request_id: string;
      channel: string;
      content: string;
      actor_owner: WireActorOwnerRef | null;
      audience: WireAudience;
    }
  | { type: "edit_message"; request_id: string; message_id: string; content: string }
  | { type: "delete_message"; request_id: string; message_id: string };

/**
 * Standalone Zod mirror of the `send_message` `ClientMsg` variant. `ClientMsg`
 * itself is a plain TS type (outgoing frames are not runtime-validated); this
 * schema exists for callers that construct/validate a `SendMessage` frame
 * before it is JSON.stringify'd onto the wire.
 */
export const SendMessageSchema = z.object({
  type: z.literal("send_message"),
  request_id: z.string(),
  channel: z.string(),
  content: z.string(),
  actor_owner: ActorOwnerRefSchema.nullable(),
  audience: AudienceSchema.default({ kind: "public" }),
});

/**
 * Standalone Zod mirror of the `pathfind` `ClientMsg` variant. `ClientMsg` itself is a plain TS
 * type (outgoing frames are not runtime-validated); this schema exists for callers that
 * construct/validate a `Pathfind` frame before it is JSON.stringify'd onto the wire. `token` is
 * nullish (absent or `null`): when present the SERVER derives the footprint from it and ignores
 * `footprint_radius` entirely — this schema does not encode that authorization, only the shape.
 */
export const PathfindSchema = z.object({
  type: z.literal("pathfind"),
  request_id: z.string(),
  scene: z.string(),
  start: z.tuple([z.number(), z.number()]),
  waypoints: z.array(z.tuple([z.number(), z.number()])),
  footprint_radius: z.number(),
  token: z.string().uuid().nullish(),
});

/** Parse + validate an inbound text frame; `null` on malformed/unknown input.
 * @param text The raw text frame received from the WebSocket.
 * @returns The parsed `ServerMsg`, or `null` if it fails `JSON.parse` or Zod validation.
 * @example
 * ```ts
 * import { parseServerMsg } from "@shadowcat/core";
 *
 * parseServerMsg('{"type":"ping"}');
 * ```
 */
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
