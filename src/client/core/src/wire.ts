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
//
// Schemas here are typed `z.ZodType<HandWrittenType>` rather than the reverse
// (`type X = z.infer<typeof XSchema>`), so the hand-written type stays the
// documentable declaration TypeDoc renders. That direction alone lets a
// SCHEMA narrower than the type pass silently: `z.ZodType<T> = expr` only
// requires `expr`'s inferred output be ASSIGNABLE to `T`, and a discriminated
// union missing an arm — or a field narrowed to a literal subtype — is still
// assignable to the wider declared type. Each such schema is therefore split
// into an unannotated `xImpl` const (its inferred type is exactly what the
// Zod expression structurally produces, not the target type read back
// through an annotation) and an exported `XSchema: z.ZodType<T> = xImpl`
// wrapper for callers; the module's test suite asserts
// `expectTypeOf<z.infer<typeof xImpl>>().toEqualTypeOf<T>()` against the
// unannotated const, which fails on both a dropped union arm and a narrowed
// field. A `z.lazy` self-referential schema (`SchemaSchema`, `DocumentSchema`)
// cannot take this split: TypeScript requires the annotation directly on the
// const a lazy callback recurses into, or the self-reference is an
// unresolvable circular type. Their guard is the weaker (but still real)
// one this pattern replaces everywhere else — missing-required-field
// detection via ordinary assignability — which is sound for them because
// neither declares a top-level discriminated union of its own to narrow.
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

/** Chat message attribution: who a message is spoken as. Mirrors `crate::chat::ActorOwnerRef`. */
export type WireActorOwnerRef =
  | {
      /** A canonical world-scoped actor document. */
      kind: "actor";
      /** The actor document id (world-pinned by `handle_send_message`). */
      actor_id: string;
    }
  | {
      /** An instanced actor addressed through its token. */
      kind: "token_instance";
      /** The token document id the instanced actor lives on. */
      token_id: string;
    };

// Unannotated impl const — see the module-level note above `z` import for why.
export const actorOwnerRefSchemaImpl = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("actor"), actor_id: z.string() }),
  z.object({ kind: z.literal("token_instance"), token_id: z.string() }),
]);
/** Validator for a `WireActorOwnerRef`. */
export const ActorOwnerRefSchema: z.ZodType<WireActorOwnerRef> = actorOwnerRefSchemaImpl;

/** The intended readership of a chat message beyond the ordinary world-readable default.
 * Carried on the `SendMessage` frame and stored verbatim in `MessageEngine`; drives the
 * document's `PermissionSet` server-side. `channel` stays a purely client-chosen label — the
 * server never validates it or derives audience from it. Mirrors `crate::chat::Audience`. */
export type WireAudience =
  | {
      /** Every world member may read (the default). */
      kind: "public";
    }
  | {
      /** Only `recipients` (plus the sender) may read. The GM reads it ONLY if their own
       * uuid is among `recipients` — not automatically. */
      kind: "whisper";
      /** User ids allowed to read; the sender is implicitly included. */
      recipients: string[];
    }
  | {
      /** Only whoever currently holds the GM role (plus the sender) may read — resolved
       * dynamically, not a frozen roster at send time. */
      kind: "gm_only";
    };

// Unannotated impl const — see the module-level note above the `z` import.
export const audienceSchemaImpl = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("public") }),
  z.object({ kind: z.literal("whisper"), recipients: z.array(z.string()) }),
  z.object({ kind: z.literal("gm_only") }),
]);
/** Validator for a `WireAudience`. */
export const AudienceSchema: z.ZodType<WireAudience> = audienceSchemaImpl;

export const ScopeSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("compendium"), pack: z.string() }),
  z.object({ kind: z.literal("world"), world_id: z.string() }),
]);

export const SourceSchema = z.object({
  id: z.string(),
  pack: z.string().nullable(),
  version: int,
});

/** Additive capability grants beyond the built-in `DocRole` floor. Each map is keyed by
 * grantee — a `DocRole` or a user id — and its values are namespaced capability strings
 * (e.g. `core:manage_embedded`). Grants widen what a role/user may do on a document; they
 * never revoke the floor. Mirrors `crate::data::document::CapabilityGrants`. */
export type WireCapabilityGrants = {
  /** Extra capabilities granted to everyone holding a given `DocRole`, keyed by role. The
   * Rust source is a map that may omit any role, so every key is optional. */
  by_role: Partial<Record<z.infer<typeof DocRoleSchema>, string[]>>;
  /** Extra capabilities granted to specific users (by id), regardless of role. User ids are
   * genuinely open, so this map stays string-keyed. */
  by_user: Record<string, string[]>;
};

// Unannotated impl const — see the module-level note above the `z` import.
export const capabilityGrantsSchemaImpl = z.object({
  by_role: z.record(DocRoleSchema, z.array(z.string())),
  by_user: z.record(z.array(z.string())),
});
/** Validator for a `CapabilityGrants`. */
export const CapabilityGrantsSchema: z.ZodType<WireCapabilityGrants> = capabilityGrantsSchemaImpl;

/** A declarative requirement: writing any field under `path_prefix` requires the
 * actor to additionally hold every capability in `caps` (on top of the structural
 * base capability for that path). Pure data — the server enforces possession and
 * never interprets the meaning of the path or the capabilities. Mirrors
 * `crate::data::document::CapabilityRequirement`. */
export type WireCapabilityRequirement = {
  /** JSON-pointer prefix the rule applies to (writes at or under it). */
  path_prefix: string;
  /** Capabilities the writer must ALL hold, on top of the structural base
   * capability for the path (`required_cap_for_path`). */
  caps: string[];
};

// Unannotated impl const — see the module-level note above the `z` import.
export const capabilityRequirementSchemaImpl = z.object({
  path_prefix: z.string(),
  caps: z.array(z.string()),
});
/** Validator for a `CapabilityRequirement`. */
export const CapabilityRequirementSchema: z.ZodType<WireCapabilityRequirement> =
  capabilityRequirementSchemaImpl;

export const CardinalitySchema = z.enum(["singleton", "multi"]);

/** A UI surface contract a module provides, with its cardinality. Mirrors
 * `crate::data::document::ContractProvide`. */
export type WireContractProvide = {
  /** The surface contract id (e.g. `shadowcat.panel`). */
  contract: string;
  /** How many providers the contract admits. */
  cardinality: z.infer<typeof CardinalitySchema>;
};

// Unannotated impl const — see the module-level note above the `z` import.
export const contractProvideSchemaImpl = z.object({
  contract: z.string(),
  cardinality: CardinalitySchema,
});
/** Validator for a `ContractProvide`. */
export const ContractProvideSchema: z.ZodType<WireContractProvide> = contractProvideSchemaImpl;

/** A module's UI contract declaration: what surface contracts it provides and which it
 * requires an active provider for. Pure data — the server validates and distributes these
 * strings; it never holds components or runs module code. Mirrors
 * `crate::data::document::ContractDeclaration`. */
export type WireContractDeclaration = {
  /** Declaring module's id. */
  module_id: string;
  /** Declaring module's version. */
  version: string;
  /** Contracts this module provides, with cardinality. */
  provides: WireContractProvide[];
  /** Contract ids this module requires an active provider for. */
  requires: string[];
};

// Unannotated impl const — see the module-level note above the `z` import.
export const contractDeclarationSchemaImpl = z.object({
  module_id: z.string(),
  version: z.string(),
  provides: z.array(ContractProvideSchema),
  requires: z.array(z.string()),
});
/** Validator for a `ContractDeclaration`. */
export const ContractDeclarationSchema: z.ZodType<WireContractDeclaration> =
  contractDeclarationSchemaImpl;

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

// Annotated directly on this const (not split into an unannotated impl + exported wrapper,
// unlike its siblings) — `z.lazy` recurses into `SchemaSchema` by name below, and TypeScript
// cannot infer this const's own type from an expression that references itself; the annotation
// is what breaks that circularity. This is judged NOT to need the split for guard strength: the
// type has no top-level discriminated union to narrow, so the assignability check the
// annotation performs still catches a dropped/renamed required field (only union-arm narrowing
// would slip past a bare annotation, and there is none here to narrow).
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

/** A world's structural schema declaration (tier-2): the `system`-band shape constraint a
 * module attaches to a doc_type's `system` subtree, GM-set and enforced server-side.
 * `schema_format` is the engine-owned vocabulary version; `version` is the module's content
 * version (provenance only). Mirrors `crate::data::document::SchemaDeclaration`. */
export type WireSchemaDeclaration = {
  /** Declaring module's id (provenance). */
  module_id: string;
  /** Declaring module's content version (provenance only). */
  version: string;
  /** Engine-owned schema-vocabulary version (`SCHEMA_FORMAT_V1`). */
  schema_format: number;
  /** The doc_type whose `system` band this schema constrains. */
  doc_type: string;
  /** Strict `/system/…` descendant pointer the schema roots at (set-time enforced). */
  subtree_pointer: string;
  /** The structural type-tree itself. */
  schema: WireSchema;
};

// Unannotated impl const — see the module-level note above the `z` import.
export const schemaDeclarationSchemaImpl = z.object({
  module_id: z.string(),
  version: z.string(),
  schema_format: int,
  doc_type: z.string(),
  subtree_pointer: z.string(),
  schema: SchemaSchema,
});
/** Validator for a `SchemaDeclaration`. */
export const SchemaDeclarationSchema: z.ZodType<WireSchemaDeclaration> =
  schemaDeclarationSchemaImpl;

/** A document's access-control set: default role, per-user role overrides, per-property
 * visibility overrides, capability grants, and an optional per-document GM-role cap. Mirrors
 * `crate::data::document::PermissionSet`. Named (rather than inferred inline via
 * `z.infer<typeof PermissionSetSchema>`) so every consumer — `WireDocument.permissions` and
 * `StampOpts.permissions` alike — resolves to one documented declaration instead of each site
 * synthesizing its own anonymous type. */
export type WirePermissionSet = {
  /** The role assigned to any user not individually listed in `users`. */
  default: z.infer<typeof DocRoleSchema>;
  /** Per-user role overrides, keyed by user id; takes precedence over `default` for that user. */
  users: Record<string, z.infer<typeof DocRoleSchema>>;
  /** Per-property visibility overrides, keyed by JSON pointer (e.g. `/system/hp`). See
   * `Access.can_see`/`filter_properties` for how these are enforced server-side. */
  property_overrides: Record<string, z.infer<typeof VisibilitySchema>>;
  /** Additive capability grants beyond the resolved role's built-in floor, by role and by user. */
  capabilities: WireCapabilityGrants;
  /** Caps the otherwise-unconditional GM see-all/edit-all short-circuit to this document's own
   * per-document role floor; `null` preserves the default unconditional GM access. See
   * `data::permission::effective_role`. */
  gm_role: z.infer<typeof DocRoleSchema> | null;
};

// Unannotated impl const — see the module-level note above the `z` import.
export const permissionSetSchemaImpl = z.object({
  default: DocRoleSchema,
  users: z.record(DocRoleSchema),
  property_overrides: z.record(VisibilitySchema),
  capabilities: CapabilityGrantsSchema,
  gm_role: DocRoleSchema.nullable(),
});
/** Validator for a `PermissionSet`. */
export const PermissionSetSchema: z.ZodType<WirePermissionSet> = permissionSetSchemaImpl;

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
  /** Opaque mergeable-content snapshot at last sync (`MergeBase`). Present on any
   * document stamped from a template (top-level or embedded, per `source`) — not restricted
   * to embedded children; absent/undefined on a document that was never stamped. */
  base?: unknown;
  /** This document's OWN `/owner` field, or `null` if unowned. Gated by `EDIT_PERMISSIONS`
   * server-side (not the bare `Owner` role), so an owner can never reassign it. A linked
   * token's EFFECTIVE owner (used for authz) can differ from this raw value — see
   * `data::permission::effective_owner`. */
  owner: string | null;
  /** The document's access-control set. See `WirePermissionSet`. */
  permissions: WirePermissionSet;
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

/** Validates a `WireDocument` envelope. `embedded` holds child documents, so the
 * schema is recursive (`z.lazy`). Callers differ on how a parse failure is handled —
 * this schema does not itself guarantee uniform treatment: `parseServerMsg` uses
 * `safeParse` and treats a failure as absent (`null`), but `applyOperation`'s
 * `"update"` branch (the `store` module) calls the throwing `.parse()` on the
 * post-image; that throw propagates out of `OptimisticClient.applyCommand`'s
 * per-op loop mid-command, leaving any sibling ops already applied earlier in the
 * same command committed while the rest of the command is abandoned, and out of
 * `WsClient.applyEvent`, which catches it, surfaces it via `onError`, and still
 * advances `nextExpected` — leaving the target document at its stale pre-update
 * value, not absent. */
// Annotated directly on this const for the same reason as `SchemaSchema` above (a `z.lazy`
// self-reference to `DocumentSchema` by name below forces the annotation here to break the
// circular type inference). Judged NOT to need the impl-const split: `WireDocument` has no
// top-level discriminated union of its own, so the annotation's assignability check still
// catches a dropped/renamed required field; only a narrowed nested field would slip past it,
// and every nested field with union structure (`permissions`, `scope`) already resolves through
// its own split, guard-covered schema.
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

/** One field-level change with its pre-image. Mirrors `crate::data::command::FieldChange`.
 * `old`/`new` are always present on the wire (the Rust struct has no
 * `skip_serializing_if` on either) and `fieldChangeSchemaImpl` rejects a frame that omits
 * either key at runtime. They stay typed optional here only because Zod v3 infers an object
 * field's declared optionality structurally: any field whose output type admits `undefined`
 * — which `z.unknown()`'s output always does — is inferred optional regardless of what a
 * `.refine()` on the whole object enforces at runtime, so the declared type cannot be
 * tightened to "required key, unknown value" against that inference rule. This is a Zod v3
 * inference limit, not a claim that the server may omit either key. */
export type WireFieldChange = {
  /** JSON pointer to the field, e.g. `/system/hp`. */
  path: string;
  /** OCC pre-image: the raw currently-stored value (`values_semantically_eq`
   * compares it at apply time server-side; a mismatch rejects the intent). */
  old?: unknown;
  /** The value to write (unused when `remove` is true). */
  new?: unknown;
  /** When true, REMOVE the object key at `path` (genuine absence) instead of
   * setting `new`. Object keys only — array-index removal is rejected server-side
   * (`remove_pointer`). Omitted on the wire when false (mirrors the server's
   * `#[serde(default, skip_serializing_if)]`). */
  remove?: boolean;
};

// Unannotated impl const — see the module-level note above the `z` import.
export const fieldChangeSchemaImpl = z
  .object({
    path: z.string(),
    // `z.unknown()` alone would accept an ABSENT key, because `undefined` satisfies
    // `unknown`. The Rust source never omits either value key, so a frame lacking one is
    // malformed. `.refine` on the whole object is what sees absence — a per-key schema
    // cannot distinguish "absent" from "present and undefined".
    old: z.unknown(),
    new: z.unknown(),
    remove: z.boolean().optional(),
  })
  .refine((v) => "old" in v, { message: "field change must carry an `old` pre-image", path: ["old"] })
  .refine((v) => "new" in v, { message: "field change must carry a `new` value", path: ["new"] });
/** Validator for a single field change carried by an `Operation`. */
export const FieldChangeSchema: z.ZodType<WireFieldChange> = fieldChangeSchemaImpl;

/** A single operation within a `Command`. Mirrors `crate::data::command::Operation`. */
export type WireOperation =
  | {
      /** Insert a whole new document. */
      op: "create";
      /** The full document to insert. */
      doc: WireDocument;
    }
  | {
      /** Remove a document (carries the full pre-image for invertibility). */
      op: "delete";
      /** The document as it existed at deletion. */
      doc: WireDocument;
    }
  | {
      /** Field-level changes against an existing document. */
      op: "update";
      /** Target document id. */
      doc_id: string;
      /** Ordered field changes, each with its OCC pre-image. */
      changes: WireFieldChange[];
    };

// Unannotated impl const — see the module-level note above the `z` import.
export const operationSchemaImpl = z.discriminatedUnion("op", [
  z.object({ op: z.literal("create"), doc: DocumentSchema }),
  z.object({ op: z.literal("delete"), doc: DocumentSchema }),
  z.object({
    op: z.literal("update"),
    doc_id: z.string(),
    changes: z.array(FieldChangeSchema),
  }),
]);
/** Validator for a single `Operation` within a `Command`. */
export const OperationSchema: z.ZodType<WireOperation> = operationSchemaImpl;

/** A command that has been assigned a per-world sequence number. Mirrors
 * `crate::data::command::Command`. */
export type WireCommand = {
  /** Per-world monotonic sequence number (the client's replay watermark). */
  seq: number;
  /** World the command applied to. */
  world_id: string;
  /** Originating user. */
  author: string;
  /** Author-side timestamp, Unix epoch milliseconds. */
  ts: number;
  /** The applied operations, in order. */
  ops: WireOperation[];
};

// Unannotated impl const — see the module-level note above the `z` import.
export const commandSchemaImpl = z.object({
  seq: int,
  world_id: z.string(),
  author: z.string(),
  ts: int,
  ops: z.array(OperationSchema),
});
/** Validator for a `Command`. */
export const CommandSchema: z.ZodType<WireCommand> = commandSchemaImpl;

/** One search result: the per-recipient-filtered document, its BM25 relevance, and a
 * highlighted snippet. Mirrors `crate::data::search::SearchHit`. */
export type WireSearchHit = {
  /** The matched document, already per-recipient filtered. */
  document: WireDocument;
  /** BM25 relevance as SQLite returns it (lower = more relevant). */
  score: number;
  /** Highlighted match snippet from the recipient's own index partition. `index_content` sweeps
   * the `doc_type` unconditionally, the document's `name`, and — through `collect_leaves` —
   * every string AND number leaf of both `engine` and `system`, so any of them can surface here
   * and in `document`. `doc_type` is client-supplied on `Create` and no charset validation
   * constrains it, so a consumer must render this as inert text and never as innerHTML. */
  snippet: string;
};

// Unannotated impl const — see the module-level note above the `z` import.
export const searchHitSchemaImpl = z.object({
  document: DocumentSchema,
  score: z.number(),
  snippet: z.string(),
});
/** Validator for a `SearchHit`. */
export const SearchHitSchema: z.ZodType<WireSearchHit> = searchHitSchemaImpl;

/** A single position sample in a `move_stream` timeline. `t_ms` is elapsed milliseconds from
 * `start_server_ms`; `pos` is the scene-coord cell-center at that instant. INVARIANT:
 * `t_ms >= 0`; samples are ordered by ascending `t_ms`. Mirrors `crate::ws::protocol::PosSample`. */
export type WireMoveStreamSample = {
  /** Elapsed time in milliseconds from the enclosing frame's `start_server_ms`. */
  t_ms: number;
  /** Scene-coordinate position (x, y) at this sample instant. */
  pos: [number, number];
};

/** A single vision-polygon sample in a `move_stream` timeline, paired with a position sample
 * by `t_ms`. Ordered `[x,y]` vertices of a visible region at this instant; multiple polygons
 * cover non-contiguous visible regions. Not necessarily convex. Sent only for the mover.
 * Mirrors `crate::ws::protocol::VisionSample`. */
export type WireMoveStreamVisionSample = {
  /** Elapsed time in milliseconds — matches the corresponding position sample's `t_ms`. */
  t_ms: number;
  /** Visibility polygons (scene coords) visible at this instant. Each polygon is an ordered
   * list of [x, y] vertices; multiple polygons cover non-contiguous visible areas. */
  polygons: [number, number][][];
};

/** The `welcome` server frame, sent right after a successful join. Carries the world's default
 * capability grants, the connecting user's world role, and the declarative capability
 * requirements so the client can replicate access resolution for advisory UI gating (the server
 * remains authoritative). A named `ServerMsg` union arm (rather than an inline object literal)
 * so it resolves to one documented declaration wherever it is referenced, including through the
 * `ws-client` module's re-export. */
export type WireWelcome = {
  /** Discriminant literal selecting the `welcome` variant of `ServerMsg`. */
  type: "welcome";
  /** The joined world. */
  world: string;
  /** The world's latest committed seq at join time. */
  current_seq: number;
  /** Server wall-clock at send, Unix epoch milliseconds. */
  server_time: number;
  /** The running server's semver (`CARGO_PKG_VERSION`). The client's load-time
   * engine-compat gate checks each external module's `engines.shadowcat` range against
   * this; delivered here (authenticated, per-session) rather than on public
   * `/api/config` to avoid disclosing the exact build to unauthenticated callers. */
  server_version: string;
  /** The world's default per-document capability grants. */
  world_default_grants: WireCapabilityGrants;
  /** The connecting user's role in this world. */
  user_role: z.infer<typeof WorldRoleSchema>;
  /** Declarative path-prefix capability requirements (advisory mirror). */
  capability_requirements: WireCapabilityRequirement[];
  /** The world's UI contract declarations, so the client can validate its loaded module
   * set against the world's declared topology. */
  contract_declarations: WireContractDeclaration[];
  /** The world's structural schema declarations (tier-2), so the client can mirror
   * expectations. Informational/parity only — tier-1 Zod validates client-side; this is
   * NOT a client enforcement gate. */
  schema_declarations: WireSchemaDeclaration[];
};

/** Every frame the server sends, discriminated by `type`. Mirrors
 * `crate::ws::protocol::ServerMsg`. */
export type ServerMsg =
  | WireWelcome
  | {
      /** A sequenced broadcast carrying the authoritative command. */
      type: "event";
      /** The committed, per-recipient-filtered command. */
      command: WireCommand;
      /** Originator's correlation token; `null` on the shared broadcast (an originator
       * confirms its own write by receiving this echo of its authored command), and
       * non-null when the write was made under an intent id, correlating this event back
       * to that specific intent. */
      intent_id: string | null;
    }
  | {
      /** An `intent` the write path refused, addressed to its originator only. */
      type: "reject";
      /** The refused intent's correlation token. */
      intent_id: string;
      /** Why it was refused. */
      reason: z.infer<typeof RejectReasonSchema>;
    }
  | {
      /** Opens a resync replay range. */
      type: "resync_begin";
      /** First seq delivered in the replay (inclusive; equals the client's requested
       * `from_seq`). */
      from_seq: number;
      /** Last seq the replay will deliver. */
      to_seq: number;
      /** Which tier serves the replay. */
      source: z.infer<typeof ResyncSourceSchema>;
    }
  | {
      /** Closes a resync replay range; live delivery resumes after this. */
      type: "resync_end";
      /** The authoritative seq after replay; live delivery resumes here. */
      current_seq: number;
    }
  | {
      /** Time calibration reply: echoes the client send time, adds the server time. */
      type: "time_pong";
      /** Echo of the ping's client send time. */
      client_t0: number;
      /** Server wall-clock at reply, Unix epoch milliseconds. */
      server_t: number;
    }
  | {
      /** Heartbeat. */
      type: "ping";
    }
  | {
      /** A non-fatal or fatal error, by code. */
      type: "error";
      /** Machine-actionable category. */
      code: z.infer<typeof WsErrorCodeSchema>;
      /** Player-presentable text (never internal details). */
      message: string;
    }
  | {
      /** Results for the `search` with this `request_id`. Documents are already filtered
       * for the recipient. */
      type: "search_result";
      /** The originating search's correlation token. */
      request_id: string;
      /** Per-recipient-filtered hits, rank order. */
      hits: WireSearchHit[];
      /** Opaque next-page token; `null` = exhausted. */
      next_cursor: string | null;
    }
  | {
      /** The `search` with this `request_id` failed. */
      type: "search_error";
      /** The failed search's correlation token. */
      request_id: string;
      /** Player-presentable failure text. */
      message: string;
    }
  | {
      /** A live subscription's refreshed top-N (full replace). Documents are already
       * filtered for the recipient. */
      type: "search_update";
      /** The live subscription's correlation token. */
      request_id: string;
      /** The refreshed, per-recipient-filtered top-N (full replace). */
      hits: WireSearchHit[];
    }
  | {
      /** A derived-state push: coalesced, per recipient, ordered after the document events
       * it reflects via `computed_at_seq`. `payload` is opaque to the transport. */
      type: "scene_derived";
      /** The subscription's correlation token. */
      request_id: string;
      /** The channel this push belongs to. */
      channel: string;
      /** The document seq this state was computed at (orders vs events). */
      computed_at_seq: number;
      /** Channel-defined derived state; opaque to the transport. `z.unknown()` infers an
       * optional property (same reasoning as `WireDocument.engine`/`.system`). */
      payload?: unknown;
    }
  | {
      /** A derived subscription failed (e.g. unknown channel). */
      type: "scene_error";
      /** The failed subscription's correlation token. */
      request_id: string;
      /** Player-presentable failure text. */
      message: string;
    }
  | {
      /** Out-of-band asset mutation notice. Carries no seq and is never buffered or
       * resynced; holders re-resolve against the record's `version`. */
      type: "asset_changed";
      /** The mutated asset's id. */
      uuid: string;
      /** What happened to it. */
      op: "replaced" | "deleted";
    }
  | {
      /** A relayed location ping: the sender's transient marker at scene coords. Out-of-band
       * (no seq, never buffered/resynced), mirroring `asset_changed`. */
      type: "scene_ping";
      /** Scene the ping landed on. */
      scene: string;
      /** Scene-coordinate x. */
      x: number;
      /** Scene-coordinate y. */
      y: number;
      /** Who pinged (senders receive their own echo). */
      user: string;
    }
  | {
      /** The route for the `pathfind` with this `request_id`: ordered cell-center scene
       * points (incl. start + goal) and the total cost in cells (client multiplies
       * `grid.distance.perCell`). `arrested` is true when an arrest region truncated the
       * route before the requested goal — the player-facing route never silently ends
       * short without telling the client why. */
      type: "path_result";
      /** The originating pathfind's correlation token. */
      request_id: string;
      /** Ordered cell-center scene points, start through goal inclusive. */
      path: [number, number][];
      /** Total route cost in cells (multiply by `grid.distance.perCell`). */
      cost: number;
      /** True when an arrest region truncated the route short of the goal. */
      arrested: boolean;
    }
  | {
      /** The `pathfind` with this `request_id` failed (unreachable / invalid request /
       * search exceeded). */
      type: "path_error";
      /** The failed pathfind's correlation token. */
      request_id: string;
      /** Player-presentable failure text. */
      message: string;
    }
  | {
      /** A `move_request` was rejected (token already moving, caller not owner, malformed
       * path, etc.). Addressed to the originating connection only; never broadcast. */
      type: "move_error";
      /** The refused move's correlation token. */
      request_id: string;
      /** Player-presentable failure text. */
      message: string;
    }
  | {
      /** A `send_message`/`edit_message`/`delete_message` was rejected. One shared variant
       * covers all three chat ops: they share a single error enum (`chat::SendMessageError`)
       * and its player-presentable `Display`; the failed op is implicit in which request
       * `request_id` belongs to. Addressed to the originating connection only; never
       * broadcast. */
      type: "chat_error";
      /** The refused chat op's correlation token. */
      request_id: string;
      /** `SendMessageError`'s player-presentable `Display` text — authorization/existence/
       * internal classes are already collapsed to a generic string there (no leak). */
      message: string;
    }
  | {
      /** Broadcast to the scene, then clipped per recipient at egress: the mover receives
       * the full trajectory and `mover_vision`; observers receive only the position samples
       * their own vision admits, with `mover_vision` nulled; a fully-occluded recipient
       * receives nothing. */
      type: "move_stream";
      /** Correlates with the originating `move_request`. */
      request_id: string;
      /** The token being moved. */
      token_id: string;
      /** The user who owns the move (mover's user id). */
      mover: string;
      /** The scene in which the move occurs. */
      scene: string;
      /** Authoritative server wall-clock time (ms) at which the animation starts.
       * INVARIANT: must be set before send so all clients sync to the same origin. */
      start_server_ms: number;
      /** Total wall-clock animation budget in milliseconds. */
      duration_ms: number;
      /** Final resting position (scene coords) after the move completes. */
      stop: [number, number];
      /** Ordered position samples along the route (t=0 is start, t=duration_ms is stop).
       * INVARIANT: non-empty; first sample t_ms == 0.0 is the starting cell-center. */
      samples: WireMoveStreamSample[];
      /** Per-sample vision polygons for the mover only. `null` for observers, who receive
       * server-clipped position samples and render against their existing authoritative fog;
       * the client computes no vision. Sending mover vision to observers would leak geometry. */
      mover_vision: WireMoveStreamVisionSample[] | null;
      /** Total terrain-weighted movement cost accumulated over the executed move.
       * Informational — no per-turn budget cap consumes it in v1. Present for the mover and
       * a GM (trusted, full information); `null` for a clipped observer, mirroring
       * `mover_vision`'s null-for-observers treatment — the authoritative cost may reflect
       * secret-region (`gm_only`) terrain the observer's clipped `samples` don't show, and
       * disclosing it would let an observer detect hidden terrain by comparing the visible
       * portion of the move against the reported total. */
      cost: number | null;
      /** `true` when the move stopped before the requested goal — wall, mask,
       * region-impassable, or region-arrest. The authoritative answer: a client cannot
       * derive it from `stop` alone, because a region-arrest on the FINAL step ends the move
       * AT the goal coordinate and so is indistinguishable from an untruncated move by
       * geometry. Present for the mover and a GM (trusted, full information); `null` for a
       * clipped observer, on the same grounds as `cost` — the observer's `samples` and
       * `stop` are already clipped to what they witnessed, so a truthful `truncated` would
       * disclose whether anything blocked the token BEYOND their vision, revealing the
       * presence of a wall or a `gm_only` region they cannot see. */
      truncated: boolean | null;
    }
  | {
      /** Terminal eviction notice: the recipient's world or account is being deleted.
       * `user: null` addresses every connection in the room (world deletion); a set id
       * addresses only that user's connections (account deletion — broadcast to every room,
       * non-targets skip it silently). The egress loop delivers this frame, sends a
       * protocol Close, and terminates the connection; the client must treat it as terminal
       * (no reconnect). */
      type: "evicted";
      /** `null` = every connection in the room; set = that user only. */
      user: string | null;
    };

// Unannotated impl const — see the module-level note above the `z` import. This is the
// finding's worked example: deleting an arm (e.g. the "reject" object below) or narrowing a
// field (e.g. `user_role: z.literal("gm")` in the "welcome" arm) makes
// `serverMsgSchemaImpl`'s inferred type a strict subset of `ServerMsg`, which the drift-guard
// test's `expectTypeOf<z.infer<typeof serverMsgSchemaImpl>>().toEqualTypeOf<ServerMsg>()`
// rejects — `toEqualTypeOf` requires exact bidirectional equality, not mere assignability.
export const serverMsgSchemaImpl = z.discriminatedUnion("type", [
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
    cost: z.number().nullable(),
    // `truncated` is declared here (rather than left to fall through) because `z.object`
    // strips keys it does not name — an omitted field is silently discarded at parse, so a
    // server field absent from this schema never reaches any consumer.
    truncated: z.boolean().nullable(),
  }),
  z.object({ type: z.literal("evicted"), user: z.string().nullable() }),
]);
/** Validator for every frame the server sends, discriminated by `type`. */
export const ServerMsgSchema: z.ZodType<ServerMsg> = serverMsgSchemaImpl;

/** The inferred TS shape of `ScopeSchema` above. */
export type WireScope = z.infer<typeof ScopeSchema>;

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
  | {
      /** Heartbeat reply. */
      type: "pong";
    }
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
       * itself and refuses on mismatch, so this field selects nothing on its own. Success
       * broadcasts `move_stream` to the scene; failure replies `move_error` to the requester
       * only. */
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
