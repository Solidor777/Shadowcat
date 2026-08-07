// Runtime validation for the WebSocket wire protocol. The compile-time wire
// types come from `@shadowcat/types` (ts-rs output); these Zod schemas validate
// inbound server frames at the trust boundary, plus outbound client-frame
// schemas (e.g. `SendMessageSchema`) that callers may opt to validate before
// sending. This module's own "wire drift guard" test suites guard them against drift from
// the Rust types.
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
/** The inferred TS shape of `ActorOwnerRefSchema` above. */
export type WireActorOwnerRef = z.infer<typeof ActorOwnerRefSchema>;

/** Mirrors `crate::chat::Audience` (chat message readership). */
export const AudienceSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("public") }),
  z.object({ kind: z.literal("whisper"), recipients: z.array(z.string()) }),
  z.object({ kind: z.literal("gm_only") }),
]);
/** The inferred TS shape of `AudienceSchema` above. */
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
/** The inferred TS shape of `CapabilityRequirementSchema` above. */
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
/** The inferred TS shape of `ContractDeclarationSchema` above. */
export type WireContractDeclaration = z.infer<typeof ContractDeclarationSchema>;

export const SchemaTypeSchema = z.enum([
  "object",
  "array",
  "string",
  "number",
  "boolean",
  "null",
]);

/** Recursive structural type-tree (tier-2). Absent fields are optional (the server
 * omits a `None` via `skip_serializing_if`). Shape only — never a value rule; the grammar
 * has no `enum`, numeric/string bounds, `pattern`, or `anyOf`/`oneOf` combinator. */
export type WireSchema = {
  /** The primitive/container kind this node matches; absent = unconstrained. */
  type?: z.infer<typeof SchemaTypeSchema>;
  /** Child schemas keyed by property name; meaningful only when `type` is `"object"`. */
  properties?: Record<string, WireSchema>;
  /** Property names that must be present; meaningful only when `type` is `"object"`. */
  required?: string[];
  /** Whether (or how) properties beyond `properties` are permitted. Absent behaves as `false`
   * (closed) — this grammar has no combinator, `pattern`, or value-range form, only shape. */
  additionalProperties?: boolean | WireSchema;
  /** The element schema; meaningful only when `type` is `"array"`. */
  items?: WireSchema;
  /** Whether `null` is also a valid value for the declared `type`. */
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
/** The inferred TS shape of `SchemaDeclarationSchema` above. */
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
  /** Document id (UUID). Client-generated at Create — this is what lets the client apply the
   * doc optimistically under the same id the server will later confirm — and immutable
   * thereafter (no field-path Update reaches `/id`). */
  id: string;
  /** Placement: `{kind:"world", world_id}` or `{kind:"compendium", pack}`. See
   * `data::document::world_of` for the world-scope extraction authz keys off of. */
  scope: z.infer<typeof ScopeSchema>;
  /** Unconstrained wire string naming the document's kind (e.g. `actor`, `scene`, `message`).
   * Real server-side structural authority applies only to the 17 engine-defined types
   * (`data::engine::is_engine_doc_type`); any other value is a legitimate
   * client-only doc_type (e.g. `item`, see `ITEM_DOC_TYPE`). */
  doc_type: string;
  /** Per-document schema-migration marker (`CURRENT_SCHEMA_VERSION`, currently
   * 1). The dispatch machinery exists but has no real migration steps yet. */
  schema_version: number;
  /** Universal display name. Redacts to `null` (never a stripped key) under a `/name`
   * property-visibility override. */
  name: string | null;
  /** Template/instance provenance (source template id, its compendium pack if any, and its
   * content version at stamp time), or `null` for a document with no source. Mirrors
   * `crate::data::document::Source`. */
  source: z.infer<typeof SourceSchema> | null;
  /** Opaque mergeable-content snapshot at last sync (`MergeBase`, `./merge`). Present on any
   * document stamped from a template (top-level or embedded, per `source`) — not restricted
   * to embedded children; absent/undefined on a document that was never stamped. */
  base?: unknown;
  /** This document's OWN `/owner` field, or `null` if unowned. Gated by `EDIT_PERMISSIONS`
   * server-side (not the bare `Owner` role), so an owner can never reassign it. A linked
   * token's EFFECTIVE owner (used for authz) can differ from this raw value — see
   * `data::permission::effective_owner`. */
  owner: string | null;
  /** The document's access-control set: default role, per-user role overrides, per-property
   * visibility overrides, capability grants, and an optional per-document GM-role cap. */
  permissions: z.infer<typeof PermissionSetSchema>;
  /** Child documents keyed by `doc_type` (e.g. an actor's inventory `item`s). Each embedded
   * doc is itself a full `WireDocument`, recursively. */
  embedded: Record<string, WireDocument[]>;
  /** Scene-entity link: the parent scene's id (or other parent); `null` for top-level docs
   * (actors, compendium entries, scenes). No capability path reaches `/parent_id`
   * (`required_cap_for_path` returns `None` for it) — immutable via field-path Update. */
  parent_id: string | null;
  /** Engine band: present iff `doc_type` is engine-defined; validated + typed via the
   * generated `*Engine` structs (`@shadowcat/types`). `z.unknown()` infers an optional
   * property, so an absent/non-engine doc_type's `engine` key is simply undefined. */
  engine?: unknown;
  /** Opaque `system` body; `z.unknown()` infers an optional property. The server enforces
   * only structural rules on it (size cap, JSON validity, optional tier-2 shape schema) —
   * it never interprets the value semantically. */
  system?: unknown;
  /** Server-set creation timestamp (ms since epoch). */
  created_at: number;
  /** Server-set last-modification timestamp (ms since epoch); advances on every applied
   * field change. */
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
/** The inferred TS shape of `SearchHitSchema` above. */
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

/** The inferred TS shape of `ScopeSchema` above. */
export type WireScope = z.infer<typeof ScopeSchema>;
/** The inferred TS shape of `FieldChangeSchema` above. */
export type WireFieldChange = z.infer<typeof FieldChangeSchema>;
/** The inferred TS shape of `OperationSchema` above. */
export type WireOperation = z.infer<typeof OperationSchema>;
/** The inferred TS shape of `CommandSchema` above. */
export type WireCommand = z.infer<typeof CommandSchema>;
/** The inferred TS shape of `ServerMsgSchema` above (the full discriminated union of inbound
 * server frames). */
export type ServerMsg = z.infer<typeof ServerMsgSchema>;

/** Client -> server frames. Plain objects (numbers, JSON.stringify-friendly). Mirrors
 * `ws::protocol::ClientMsg` variant-by-variant; each
 * variant's per-field doc below cites that Rust doc as the source of truth. */
export type ClientMsg =
  | {
      /** First frame after upgrade: names the world and the client's last known seq. */
      type: "hello";
      /** The world to join. */
      world: string;
      /** Highest seq the client has applied; `null` = cold start (full sync). */
      last_seq: number | null;
    }
  | {
      /** A proposed write: a client-chosen `intent_id` plus the ops, applied all-or-nothing
       * through the one write path. Success broadcasts an `event`; failure returns `reject`. */
      type: "intent";
      /** Client-chosen correlation token echoed on `event`/`reject`. */
      intent_id: string;
      /** The proposed operations, applied all-or-nothing. */
      ops: WireOperation[];
    }
  | {
      /** Explicit gap recovery from the client's sequence guard. */
      type: "resync_request";
      /** The first seq to replay, INCLUSIVE — the next seq the client has not yet applied. */
      from_seq: number;
    }
  | {
      /** Time calibration ping carrying the client's send timestamp. */
      type: "time_ping";
      /** Client send timestamp, echoed back in `time_pong`. */
      client_t0: number;
    }
  | { /** Heartbeat reply. */ type: "pong" }
  | {
      /** A full-text search request, correlated by `request_id`. When `subscribe` is true, the
       * initial `search_result` is followed by `search_update`s on change. */
      type: "search";
      /** Correlation token for the result/update/error frames. */
      request_id: string;
      /** Raw query text (sanitized server-side into an FTS MATCH). */
      query: string;
      /** Maximum hits per page. */
      limit: number;
      /** Opaque page token from a prior `search_result`; absent = first page. */
      cursor?: string;
      /** True = keep a live top-N subscription pushing `search_update`s. */
      subscribe: boolean;
    }
  | {
      /** Cancel a live search subscription (idempotent; unknown id ignored). */
      type: "unsubscribe";
      /** The live search to cancel. */
      request_id: string;
    }
  | {
      /** Subscribe to a derived scene channel (e.g. "vision"); unknown channels yield
       * `scene_error`. */
      type: "scene_subscribe";
      /** Correlation token for the derived pushes/errors. */
      request_id: string;
      /** Channel name (e.g. "vision"). */
      channel: string;
      /**
       * GM-only see-as-player override: a member's user id, so the channel is
       * derived from THAT user's view instead of the caller's. Omit for the
       * caller's own view. Authorized entirely server-side
       * (`ws::conn::egress_loop`'s `SceneSubscribe` handling): a non-GM caller gets
       * `scene_error` "not authorized to view as another user", and a target
       * who is not a member of the world gets `scene_error` "target user is
       * not a member of this world". The target's role is resolved from the
       * server's own membership record — a client-supplied role or scope is
       * never trusted, which is what makes this the player-to-player access
       * boundary.
       */
      as_user?: string;
    }
  | {
      /** Cancel a derived subscription by request id. */
      type: "scene_unsubscribe";
      /** The derived subscription to cancel. */
      request_id: string;
    }
  | {
      /** A transient location ping at scene coords, relayed out-of-band with the sender
       * stamped server-side; never sequenced, logged, or a document. Coordinates are not
       * validated; the scene must exist in this world and grant the sender READ (silent
       * drop otherwise), and the frame is rate-limited per connection. */
      type: "scene_ping";
      /** Scene the ping lands on (must grant the sender READ). */
      scene: string;
      /** Scene-coordinate x. */
      x: number;
      /** Scene-coordinate y. */
      y: number;
    }
  | {
      /** A one-shot grid pathfinding request, correlated by `request_id`. When `token` is
       * given, the server derives the footprint from that token's document and IGNORES
       * `footprint_radius` — so a route preview and the authoritative move gate cannot
       * disagree about the mover's size; the client-supplied radius is honored only when
       * `token` is omitted, as an explicitly hypothetical preview. */
      type: "pathfind";
      /** Correlation token for `path_result`/`path_error`. */
      request_id: string;
      /** Scene to route on. */
      scene: string;
      /** Route origin, scene coords. */
      start: [number, number];
      /** Intermediate points; the LAST element is the goal, scene coords. */
      waypoints: [number, number][];
      /** Mover radius in grid units; ignored server-side when `token` is given. */
      footprint_radius: number;
      /** The token the route is for; when present, also the source of the authoritative
       * footprint (see the variant doc). */
      token?: string;
    }
  | {
      /** A server-authoritative move request for a token the caller controls. `scene` is
       * checked only for agreement — the server DERIVES the acting scene from the token
       * itself and refuses on mismatch, so this field selects nothing on its own (see the
       * derive-from-token invariant, `shadowcat-codebase-realtime-sync`). Success broadcasts
       * `move_stream` to the scene; failure replies `move_error` to the requester only. */
      type: "move_request";
      /** Correlation token for `move_error` (success echoes via the broadcast `move_stream`). */
      request_id: string;
      /** The scene the token is expected to be on; checked, not selected — see the variant
       * doc. */
      scene: string;
      /** The token to move (must be effectively owned by the requester). */
      token_id: string;
      /** Ordered cell-center scene points: start … goal (inclusive). */
      path: [number, number][];
    }
  | {
      /** Author a chat message. The server sanitizes `content` and constructs the stored
       * message doc itself — this is the sole message-authoring path; a client `Create` of
       * a `message` doc is rejected. Success is confirmed only by the broadcast `event`
       * echo (no ack frame); a rejection replies `chat_error` correlated by `request_id`. */
      type: "send_message";
      /** Correlation token for a `chat_error` rejection. */
      request_id: string;
      /** Target channel id. */
      channel: string;
      /** Raw message text (sanitized server-side). */
      content: string;
      /** Optional in-character attribution; authz-checked server-side. */
      actor_owner: WireActorOwnerRef | null;
      /** Visibility policy (public / gm-only / whisper). */
      audience: WireAudience;
    }
  | {
      /** Edit an existing message the requester owns (or any, if GM); channel/audience are
       * frozen. Same asymmetric reply protocol as `send_message`. */
      type: "edit_message";
      /** Correlation token for a `chat_error` rejection. */
      request_id: string;
      /** The message to edit. */
      message_id: string;
      /** Replacement text (re-sanitized server-side). */
      content: string;
    }
  | {
      /** Soft-delete a message the requester owns (or any, if GM): the doc stays in the
       * sequenced log as a tombstone (content cleared, `deleted_at` set). Same asymmetric
       * reply protocol as `send_message`. */
      type: "delete_message";
      /** Correlation token for a `chat_error` rejection. */
      request_id: string;
      /** The message to tombstone. */
      message_id: string;
    };

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
