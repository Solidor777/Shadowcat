import { describe, it, expect, expectTypeOf } from "vitest";
import { z } from "zod";
import type * as Ts from "@shadowcat/types";
import {
  parseServerMsg,
  DocRoleSchema,
  VisibilitySchema,
  WorldRoleSchema,
  RejectReasonSchema,
  ResyncSourceSchema,
  WsErrorCodeSchema,
  SendMessageSchema,
  PathfindSchema,
  DocumentSchema,
  SchemaTypeSchema,
  SchemaDeclarationSchema,
  FieldChangeSchema,
  CapabilityGrantsSchema,
  actorOwnerRefSchemaImpl,
  audienceSchemaImpl,
  capabilityGrantsSchemaImpl,
  capabilityRequirementSchemaImpl,
  contractProvideSchemaImpl,
  contractDeclarationSchemaImpl,
  schemaDeclarationSchemaImpl,
  permissionSetSchemaImpl,
  fieldChangeSchemaImpl,
  operationSchemaImpl,
  commandSchemaImpl,
  searchHitSchemaImpl,
  serverMsgSchemaImpl,
  type ServerMsg,
  type ClientMsg,
  type WireOperation,
  type WireAudience,
  type WireActorOwnerRef,
  type WireCapabilityGrants,
  type WireCapabilityRequirement,
  type WireContractProvide,
  type WireContractDeclaration,
  type WireSchemaDeclaration,
  type WirePermissionSet,
  type WireFieldChange,
  type WireCommand,
  type WireSearchHit,
} from "./wire";

// Drift guard. Exact field-by-field type equality fights Zod's inference
// (`z.unknown()` → optional, i64 `bigint` → `number`), so the type-level guard
// covers the highest-value drift: enum membership and message/op discriminants.
// A renamed/added/removed enum variant or message tag in the Rust types fails
// typecheck here. Field-shape drift is exercised by the integration tests.
describe("wire drift guard — enums", () => {
  it("DocRole", () => {
    expectTypeOf<z.infer<typeof DocRoleSchema>>().toEqualTypeOf<Ts.DocRole>();
  });
  it("Visibility", () => {
    expectTypeOf<
      z.infer<typeof VisibilitySchema>
    >().toEqualTypeOf<Ts.Visibility>();
  });
  it("WorldRole", () => {
    expectTypeOf<
      z.infer<typeof WorldRoleSchema>
    >().toEqualTypeOf<Ts.WorldRole>();
  });
  it("RejectReason", () => {
    expectTypeOf<
      z.infer<typeof RejectReasonSchema>
    >().toEqualTypeOf<Ts.RejectReason>();
  });
  it("ResyncSource", () => {
    expectTypeOf<
      z.infer<typeof ResyncSourceSchema>
    >().toEqualTypeOf<Ts.ResyncSource>();
  });
  it("WsErrorCode", () => {
    expectTypeOf<
      z.infer<typeof WsErrorCodeSchema>
    >().toEqualTypeOf<Ts.WsErrorCode>();
  });
});

describe("wire drift guard — message discriminants", () => {
  it("ServerMsg type tags", () => {
    expectTypeOf<ServerMsg["type"]>().toEqualTypeOf<Ts.ServerMsg["type"]>();
  });
  it("Welcome capability fields match ts-rs", () => {
    type W = Extract<ServerMsg, { type: "welcome" }>;
    type T = Extract<Ts.ServerMsg, { type: "welcome" }>;
    // i64 fields (current_seq/server_time) are intentionally number vs bigint;
    // guard only the capability fields, which carry no i64 scalar mismatch.
    expectTypeOf<W["user_role"]>().toEqualTypeOf<T["user_role"]>();
    expectTypeOf<W["capability_requirements"]>().toEqualTypeOf<
      T["capability_requirements"]
    >();
    expectTypeOf<W["contract_declarations"]>().toEqualTypeOf<
      T["contract_declarations"]
    >();
  });
  it("ClientMsg type tags", () => {
    expectTypeOf<ClientMsg["type"]>().toEqualTypeOf<Ts.ClientMsg["type"]>();
  });
  it("Operation op tags", () => {
    expectTypeOf<WireOperation["op"]>().toEqualTypeOf<Ts.Operation["op"]>();
  });
  it("Welcome schema_declarations match ts-rs", () => {
    type W = Extract<ServerMsg, { type: "welcome" }>;
    type T = Extract<Ts.ServerMsg, { type: "welcome" }>;
    expectTypeOf<W["schema_declarations"]>().toEqualTypeOf<T["schema_declarations"]>();
  });
  it("combat_error is a ServerMsg discriminant", () => {
    type Found = Extract<ServerMsg["type"], "combat_error">;
    expectTypeOf<Found>().toEqualTypeOf<"combat_error">();
  });
  it("combat_advance is assignable as a ClientMsg", () => {
    const frame: ClientMsg = {
      type: "combat_advance",
      request_id: "00000000-0000-0000-0000-000000000001",
      combat_id: "00000000-0000-0000-0000-000000000002",
    };
    expectTypeOf(frame).toMatchTypeOf<ClientMsg>();
  });
});

// Non-vacuous schema/type guard: asserts each unannotated `xImpl` const's OWN inferred type
// against its hand-written type, rather than `z.infer<typeof XSchema>` (which reads back the
// exported const's own `z.ZodType<T>` annotation and would trivially pass regardless of what the
// schema actually validates). A dropped discriminated-union arm or a field narrowed to a
// literal subtype fails `toEqualTypeOf` here.
describe("wire drift guard — non-vacuous schema/type assertions", () => {
  it("ActorOwnerRef", () => {
    expectTypeOf<z.infer<typeof actorOwnerRefSchemaImpl>>().toEqualTypeOf<WireActorOwnerRef>();
  });
  it("Audience", () => {
    expectTypeOf<z.infer<typeof audienceSchemaImpl>>().toEqualTypeOf<WireAudience>();
  });
  it("CapabilityGrants", () => {
    expectTypeOf<z.infer<typeof capabilityGrantsSchemaImpl>>().toEqualTypeOf<WireCapabilityGrants>();
  });
  it("CapabilityRequirement", () => {
    expectTypeOf<
      z.infer<typeof capabilityRequirementSchemaImpl>
    >().toEqualTypeOf<WireCapabilityRequirement>();
  });
  it("ContractProvide", () => {
    expectTypeOf<z.infer<typeof contractProvideSchemaImpl>>().toEqualTypeOf<WireContractProvide>();
  });
  it("ContractDeclaration", () => {
    expectTypeOf<
      z.infer<typeof contractDeclarationSchemaImpl>
    >().toEqualTypeOf<WireContractDeclaration>();
  });
  it("SchemaDeclaration", () => {
    expectTypeOf<
      z.infer<typeof schemaDeclarationSchemaImpl>
    >().toEqualTypeOf<WireSchemaDeclaration>();
  });
  it("PermissionSet", () => {
    expectTypeOf<z.infer<typeof permissionSetSchemaImpl>>().toEqualTypeOf<WirePermissionSet>();
  });
  it("FieldChange", () => {
    expectTypeOf<z.infer<typeof fieldChangeSchemaImpl>>().toEqualTypeOf<WireFieldChange>();
  });
  it("Operation", () => {
    expectTypeOf<z.infer<typeof operationSchemaImpl>>().toEqualTypeOf<WireOperation>();
  });
  it("Command", () => {
    expectTypeOf<z.infer<typeof commandSchemaImpl>>().toEqualTypeOf<WireCommand>();
  });
  it("SearchHit", () => {
    expectTypeOf<z.infer<typeof searchHitSchemaImpl>>().toEqualTypeOf<WireSearchHit>();
  });
  // The finding's worked example: this is the one whose non-vacuity was hand-verified by
  // temporarily deleting the "reject" arm from `serverMsgSchemaImpl` and confirming this
  // assertion fails (see the buddy-fix report for the pasted failure/restore).
  it("ServerMsg", () => {
    expectTypeOf<z.infer<typeof serverMsgSchemaImpl>>().toEqualTypeOf<ServerMsg>();
  });
});

describe("wire drift guard — schema registry", () => {
  it("SchemaType enum", () => {
    expectTypeOf<z.infer<typeof SchemaTypeSchema>>().toEqualTypeOf<Ts.SchemaType>();
  });
  it("SchemaDeclaration shape", () => {
    // module_id/version/doc_type/subtree_pointer are strings, schema_format u32.
    expectTypeOf<
      z.infer<typeof SchemaDeclarationSchema>["subtree_pointer"]
    >().toEqualTypeOf<Ts.SchemaDeclaration["subtree_pointer"]>();
  });
});

describe("parseServerMsg", () => {
  it("validates a well-formed frame", () => {
    const m = parseServerMsg(
      JSON.stringify({
        type: "welcome",
        world: "w",
        current_seq: 0,
        server_time: 1,
        server_version: "0.0.0-test",
        world_default_grants: { by_role: {}, by_user: {} },
        user_role: "player",
        capability_requirements: [],
        contract_declarations: [],
        schema_declarations: [],
      }),
    );
    expect(m?.type).toBe("welcome");
  });

  it("parses welcome capability fields", () => {
    const m = parseServerMsg(
      JSON.stringify({
        type: "welcome",
        world: "w",
        current_seq: 0,
        server_time: 1,
        server_version: "0.0.0-test",
        world_default_grants: { by_role: { owner: ["core:manage_embedded"] }, by_user: {} },
        user_role: "gm",
        capability_requirements: [{ path_prefix: "/system/vision", caps: ["dnd5e:gm_vision"] }],
        contract_declarations: [],
        schema_declarations: [],
      }),
    );
    expect(m?.type).toBe("welcome");
    if (m?.type === "welcome") {
      expect(m.user_role).toBe("gm");
      expect(m.capability_requirements[0].path_prefix).toBe("/system/vision");
      expect(m.world_default_grants.by_role.owner).toEqual(["core:manage_embedded"]);
    }
  });

  it("parses a search_result frame", () => {
    const m = parseServerMsg(
      JSON.stringify({
        type: "search_result",
        request_id: "r1",
        hits: [],
        next_cursor: null,
      }),
    );
    expect(m?.type).toBe("search_result");
  });

  it("parses a search_update frame", () => {
    const m = parseServerMsg(
      JSON.stringify({ type: "search_update", request_id: "r1", hits: [] }),
    );
    expect(m?.type).toBe("search_update");
  });

  it("parses an inbound scene_ping frame", () => {
    const m = parseServerMsg(JSON.stringify({ type: "scene_ping", scene: "s1", x: 12, y: 34, user: "u1" }));
    expect(m?.type).toBe("scene_ping");
    if (m?.type === "scene_ping") {
      expect(m.x).toBe(12);
      expect(m.user).toBe("u1");
    }
  });

  it("parses path_result and path_error server frames", () => {
    const ok = parseServerMsg(
      JSON.stringify({
        type: "path_result",
        request_id: "00000000-0000-0000-0000-000000000001",
        path: [[50, 50], [150, 50]],
        cost: 2,
        arrested: false,
        truncated: false,
      }),
    );
    expect(ok?.type).toBe("path_result");
    const err = parseServerMsg(
      JSON.stringify({
        type: "path_error",
        request_id: "00000000-0000-0000-0000-000000000001",
        message: "unreachable",
      }),
    );
    expect(err?.type).toBe("path_error");
  });

  it("parses a well-formed move_stream frame", () => {
    const frame = {
      type: "move_stream",
      request_id: "00000000-0000-0000-0000-000000000001",
      token_id: "00000000-0000-0000-0000-000000000002",
      mover: "00000000-0000-0000-0000-000000000003",
      scene: "00000000-0000-0000-0000-000000000004",
      start_server_ms: 1000.0,
      duration_ms: 500.0,
      stop: [100.0, 200.0],
      samples: [{ t_ms: 0.0, pos: [0.0, 0.0] }, { t_ms: 500.0, pos: [100.0, 200.0] }],
      mover_vision: [
        { t_ms: 0.0, polygons: [[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]] },
      ],
      mover_light: [
        { t_ms: 0.0, pos: [0.0, 0.0], bright: 200, dim: 600, color: 0xffd9a0, intensity: 1, falloff: "linear" as const, polygons: [[[-600, -600], [600, -600], [600, 600]]] },
      ],
      cost: 3.5,
      truncated: true,
    };
    const m = parseServerMsg(JSON.stringify(frame));
    expect(m).not.toBeNull();
    expect(m?.type).toBe("move_stream");
    if (m?.type === "move_stream") {
      expect(m.stop).toEqual([100.0, 200.0]);
      expect(m.samples).toHaveLength(2);
      expect(m.mover_vision).toHaveLength(1);
      // A field absent from the schema is stripped by `z.object`, silently and with no parse
      // error, so asserting the VALUE survived is the only thing that catches an omitted key.
      expect(m.truncated).toBe(true);
      const vision = m.mover_vision;
      if (vision) expect(vision[0].polygons[0]).toHaveLength(3);
      // Same rule for the carried-light timeline: every field is asserted by value.
      expect(m.mover_light).toEqual([
        { t_ms: 0.0, pos: [0.0, 0.0], bright: 200, dim: 600, color: 0xffd9a0, intensity: 1, falloff: "linear" as const, polygons: [[[-600, -600], [600, -600], [600, 600]]] },
      ]);
    }
  });

  it("parses a move_stream frame with null mover_vision (observer path)", () => {
    const frame = {
      type: "move_stream",
      request_id: "00000000-0000-0000-0000-000000000001",
      token_id: "00000000-0000-0000-0000-000000000002",
      mover: "00000000-0000-0000-0000-000000000003",
      scene: "00000000-0000-0000-0000-000000000004",
      start_server_ms: 1000.0,
      duration_ms: 500.0,
      stop: [100.0, 200.0],
      samples: [{ t_ms: 0.0, pos: [0.0, 0.0] }],
      mover_vision: null,
      // Nothing of the light reached this recipient: the server nulls the whole timeline.
      mover_light: null,
      cost: 1.0,
      truncated: false,
    };
    const m = parseServerMsg(JSON.stringify(frame));
    expect(m).not.toBeNull();
    expect(m?.type).toBe("move_stream");
    if (m?.type === "move_stream") {
      expect(m.mover_vision).toBeNull();
      expect(m.mover_light).toBeNull();
    }
  });

  it("parses a move_stream frame with null cost (clipped observer path)", () => {
    const frame = {
      type: "move_stream",
      request_id: "00000000-0000-0000-0000-000000000001",
      token_id: "00000000-0000-0000-0000-000000000002",
      mover: "00000000-0000-0000-0000-000000000003",
      scene: "00000000-0000-0000-0000-000000000004",
      start_server_ms: 1000.0,
      duration_ms: 500.0,
      stop: [100.0, 200.0],
      samples: [{ t_ms: 0.0, pos: [0.0, 0.0] }],
      mover_vision: null,
      mover_light: null,
      // A clipped observer never learns the authoritative cost — it may reflect
      // secret-region terrain their clipped samples don't reveal.
      cost: null,
      // Withheld on the same grounds: their samples and stop are already clipped, so the flag
      // would disclose whether something stopped the token beyond their vision.
      truncated: null,
    };
    const m = parseServerMsg(JSON.stringify(frame));
    expect(m).not.toBeNull();
    expect(m?.type).toBe("move_stream");
    if (m?.type === "move_stream") {
      expect(m.cost).toBeNull();
      expect(m.truncated).toBeNull();
    }
  });

  it("parses a glow-only move_stream frame (empty samples, admitted light) and keeps the light's photometry", () => {
    const frame = {
      type: "move_stream",
      request_id: "00000000-0000-0000-0000-000000000001",
      token_id: "00000000-0000-0000-0000-000000000002",
      mover: "00000000-0000-0000-0000-000000000003",
      scene: "00000000-0000-0000-0000-000000000004",
      start_server_ms: 1000.0,
      duration_ms: 500.0,
      stop: [100.0, 200.0],
      // The token never entered this recipient's sight; only its glow did.
      samples: [],
      mover_vision: null,
      mover_light: [
        { t_ms: 500.0, pos: [100.0, 200.0], bright: 200, dim: 600, color: 0xffd9a0, intensity: 0.8, falloff: "quadratic", polygons: [[[-500, -400], [700, -400], [700, 800]]] },
      ],
      cost: null,
      truncated: null,
    };
    const m = parseServerMsg(JSON.stringify(frame));
    expect(m).not.toBeNull();
    if (m?.type === "move_stream") {
      expect(m.samples).toEqual([]);
      // Asserted by value: `z.object` strips an undeclared key silently.
      expect(m.mover_light).toEqual([
        { t_ms: 500.0, pos: [100.0, 200.0], bright: 200, dim: 600, color: 0xffd9a0, intensity: 0.8, falloff: "quadratic", polygons: [[[-500, -400], [700, -400], [700, 800]]] },
      ]);
    }
  });

  it("rejects a light sample with an unknown falloff spelling", () => {
    const frame = {
      type: "move_stream",
      request_id: "00000000-0000-0000-0000-000000000001",
      token_id: "00000000-0000-0000-0000-000000000002",
      mover: "00000000-0000-0000-0000-000000000003",
      scene: "00000000-0000-0000-0000-000000000004",
      start_server_ms: 1000.0,
      duration_ms: 500.0,
      stop: [100.0, 200.0],
      samples: [],
      mover_vision: null,
      mover_light: [
        { t_ms: 0.0, pos: [0.0, 0.0], bright: 200, dim: 600, color: 0xffd9a0, intensity: 1, falloff: "cubic", polygons: [] },
      ],
      cost: null,
      truncated: null,
    };
    expect(parseServerMsg(JSON.stringify(frame))).toBeNull();
  });

  it("rejects a move_stream frame missing samples", () => {
    const frame = {
      type: "move_stream",
      request_id: "00000000-0000-0000-0000-000000000001",
      token_id: "00000000-0000-0000-0000-000000000002",
      mover: "00000000-0000-0000-0000-000000000003",
      scene: "00000000-0000-0000-0000-000000000004",
      start_server_ms: 1000.0,
      duration_ms: 500.0,
      stop: [100.0, 200.0],
      // samples intentionally omitted
      mover_vision: null,
    };
    expect(parseServerMsg(JSON.stringify(frame))).toBeNull();
  });

  it("returns null on malformed or unknown frames", () => {
    expect(parseServerMsg("{not json")).toBeNull();
    expect(parseServerMsg(JSON.stringify({ type: "nope" }))).toBeNull();
    expect(parseServerMsg(JSON.stringify({ type: "welcome" }))).toBeNull();
  });

  it("round-trips parent_id on a create op's document (scene-entity link)", () => {
    const doc = (id: string, parent_id: string | null): unknown => ({
      id,
      scope: { kind: "world", world_id: "w1" },
      doc_type: "token",
      schema_version: 1,
      name: null,
      source: null,
      owner: null,
      permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
      embedded: {},
      parent_id,
      engine: { x: 0, y: 0, w: 0, h: 0, rotation: 0, visual: null, actor_id: null, overrides: null, face: null },
      system: {},
      created_at: 0,
      updated_at: 0,
    });
    const m = parseServerMsg(
      JSON.stringify({
        type: "event",
        intent_id: null,
        command: {
          seq: 1,
          world_id: "w1",
          author: "a",
          ts: 0,
          ops: [
            { op: "create", doc: doc("t1", "scene-1") },
            { op: "create", doc: doc("s1", null) },
          ],
        },
      }),
    );
    expect(m?.type).toBe("event");
    if (m?.type === "event") {
      const [tokenOp, sceneOp] = m.command.ops;
      if (tokenOp.op === "create") expect(tokenOp.doc.parent_id).toBe("scene-1");
      if (sceneOp.op === "create") expect(sceneOp.doc.parent_id).toBeNull();
    }
  });
});

// Table-driven exhaustiveness: keyed on `Record<ServerMsg["type"], unknown>`, so a future
// `ServerMsg` tag fails to COMPILE here until a fixture is added — coverage cannot silently
// fall behind the union again the way it did for `reject`/`resync_begin`/`time_pong`/`error`/
// `ping`. Each fixture is the smallest frame that satisfies `ServerMsgSchema` for its tag.
describe("parseServerMsg — exhaustive per-tag coverage", () => {
  const minimalFrames: Record<ServerMsg["type"], unknown> = {
    welcome: {
      type: "welcome",
      world: "w",
      current_seq: 0,
      server_time: 0,
      server_version: "0.0.0-test",
      world_default_grants: { by_role: {}, by_user: {} },
      user_role: "player",
      capability_requirements: [],
      contract_declarations: [],
      schema_declarations: [],
    },
    event: {
      type: "event",
      intent_id: null,
      command: { seq: 1, world_id: "w", author: "a", ts: 0, ops: [] },
    },
    reject: { type: "reject", intent_id: "i1", reason: "forbidden" },
    resync_begin: { type: "resync_begin", from_seq: 0, to_seq: 0, source: "buffer" },
    resync_end: { type: "resync_end", current_seq: 0 },
    time_pong: { type: "time_pong", client_t0: 0, server_t: 0 },
    ping: { type: "ping" },
    error: { type: "error", code: "internal", message: "x" },
    search_result: { type: "search_result", request_id: "r", hits: [], next_cursor: null },
    search_error: { type: "search_error", request_id: "r", message: "x" },
    search_update: { type: "search_update", request_id: "r", hits: [] },
    scene_derived: { type: "scene_derived", request_id: "r", channel: "c", computed_at_seq: 0 },
    scene_error: { type: "scene_error", request_id: "r", message: "x" },
    asset_changed: { type: "asset_changed", uuid: "u", op: "replaced", version: 1 },
    scene_ping: { type: "scene_ping", scene: "s", x: 0, y: 0, user: "u" },
    path_result: { type: "path_result", request_id: "r", path: [], cost: 0, arrested: false, truncated: false },
    path_error: { type: "path_error", request_id: "r", message: "x" },
    move_error: { type: "move_error", request_id: "r", message: "x" },
    chat_error: { type: "chat_error", request_id: "r", message: "x" },
    combat_error: { type: "combat_error", request_id: "r", message: "x" },
    move_stream: {
      type: "move_stream",
      request_id: "r",
      token_id: "t",
      mover: "m",
      scene: "s",
      start_server_ms: 0,
      duration_ms: 0,
      stop: [0, 0],
      samples: [{ t_ms: 0, pos: [0, 0] }],
      mover_vision: null,
      mover_light: null,
      cost: null,
      truncated: null,
    },
    evicted: { type: "evicted", user: null },
  };

  for (const [type, frame] of Object.entries(minimalFrames)) {
    it(`parses a minimal "${type}" frame`, () => {
      const m = parseServerMsg(JSON.stringify(frame));
      expect(m).not.toBeNull();
      expect(m?.type).toBe(type);
    });
  }
});

describe("DocumentSchema — envelope name + engine band", () => {
  const base = {
    id: "00000000-0000-0000-0000-000000000001",
    scope: { kind: "world", world_id: "w1" },
    doc_type: "actor",
    schema_version: 1,
    source: null,
    owner: null,
    permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
    embedded: {},
    parent_id: null,
    system: {},
    created_at: 0,
    updated_at: 0,
  };

  it("parses a document carrying a real name + engine body", () => {
    const parsed = DocumentSchema.parse({ ...base, name: "Strahd", engine: { displayName: "Vampire" } });
    expect(parsed.name).toBe("Strahd");
    expect(parsed.engine).toEqual({ displayName: "Vampire" });
  });

  it("parses a redacted document — name and engine nulled, never stripped", () => {
    const parsed = DocumentSchema.parse({ ...base, name: null, engine: null });
    expect(parsed.name).toBeNull();
    expect(parsed.engine).toBeNull();
  });

  it("rejects a document with no `name` key at all", () => {
    const { name: _name, ...withoutName } = { ...base, name: null, engine: {} };
    expect(DocumentSchema.safeParse(withoutName).success).toBe(false);
  });

  it("parses a document carrying a base snapshot", () => {
    const parsed = DocumentSchema.parse({
      ...base, name: "Inst", engine: {},
      base: { name: "Tmpl", engine: null, system: { hp: 1 }, embedded: {} },
    });
    expect((parsed.base as { name: string }).name).toBe("Tmpl");
  });

  it("parses a document with base absent or null (unstamped)", () => {
    expect(DocumentSchema.parse({ ...base, name: null, engine: null }).base).toBeUndefined();
    expect(DocumentSchema.parse({ ...base, name: null, engine: null, base: null }).base).toBeNull();
  });
});

describe("FieldChangeSchema", () => {
  it("rejects a frame that omits the pre-image key", () => {
    expect(FieldChangeSchema.safeParse({ path: "/system/hp", new: 3 }).success).toBe(false);
  });

  it("rejects a frame that omits the new-value key", () => {
    expect(FieldChangeSchema.safeParse({ path: "/system/hp", old: 1 }).success).toBe(false);
  });

  it("rejects a frame carrying only a path", () => {
    expect(FieldChangeSchema.safeParse({ path: "/system/hp" }).success).toBe(false);
  });

  it("accepts an explicit null pre-image, which is a real value for a new key", () => {
    expect(
      FieldChangeSchema.safeParse({ path: "/system/hp", old: null, new: 3 }).success,
    ).toBe(true);
  });

  it("accepts a removal, where new is conventionally null", () => {
    expect(
      FieldChangeSchema.safeParse({ path: "/system/hp", old: 1, new: null, remove: true })
        .success,
    ).toBe(true);
  });
});

describe("CapabilityGrantsSchema", () => {
  it("accepts the three document roles as grant keys", () => {
    expect(
      CapabilityGrantsSchema.safeParse({
        by_role: { owner: ["core:manage_embedded"], observer: [], none: [] },
        by_user: {},
      }).success,
    ).toBe(true);
  });

  it("accepts a partial role map, matching a Rust map that omits roles", () => {
    expect(
      CapabilityGrantsSchema.safeParse({ by_role: { owner: [] }, by_user: {} }).success,
    ).toBe(true);
  });

  it("rejects an unknown role key", () => {
    expect(
      CapabilityGrantsSchema.safeParse({ by_role: { admin: [] }, by_user: {} }).success,
    ).toBe(false);
  });

  it("still accepts an arbitrary user id as a by_user key", () => {
    expect(
      CapabilityGrantsSchema.safeParse({
        by_role: {},
        by_user: { "00000000-0000-0000-0000-000000000001": ["core:delete"] },
      }).success,
    ).toBe(true);
  });
});

describe("wire drift guard — Audience", () => {
  it("Audience kind tags", () => {
    expectTypeOf<WireAudience["kind"]>().toEqualTypeOf<Ts.Audience["kind"]>();
  });
});

describe("SendMessageSchema", () => {
  it("parses a send_message frame + actor_owner ref", () => {
    expect(
      SendMessageSchema.parse({
        type: "send_message",
        request_id: "00000000-0000-0000-0000-0000000000aa",
        channel: "all",
        content: "hi",
        actor_owner: {
          kind: "actor",
          actor_id: "00000000-0000-0000-0000-000000000001",
        },
      }).actor_owner?.kind,
    ).toBe("actor");
  });

  it("requires request_id", () => {
    expect(
      SendMessageSchema.safeParse({
        type: "send_message",
        channel: "all",
        content: "hi",
        actor_owner: null,
      }).success,
    ).toBe(false);
  });

  it("defaults audience to public when omitted", () => {
    const parsed = SendMessageSchema.parse({
      type: "send_message",
      request_id: "00000000-0000-0000-0000-0000000000aa",
      channel: "all",
      content: "hi",
      actor_owner: null,
    });
    expect(parsed.audience).toEqual({ kind: "public" });
  });

  it("parses a whisper audience with recipients", () => {
    const parsed = SendMessageSchema.parse({
      type: "send_message",
      request_id: "00000000-0000-0000-0000-0000000000aa",
      channel: "whispers",
      content: "psst",
      actor_owner: null,
      audience: {
        kind: "whisper",
        recipients: ["00000000-0000-0000-0000-000000000001"],
      },
    });
    expect(parsed.audience).toEqual({
      kind: "whisper",
      recipients: ["00000000-0000-0000-0000-000000000001"],
    });
  });

  it("parses a gm_only audience", () => {
    const parsed = SendMessageSchema.parse({
      type: "send_message",
      request_id: "00000000-0000-0000-0000-0000000000aa",
      channel: "gm",
      content: "for your eyes only",
      actor_owner: null,
      audience: { kind: "gm_only" },
    });
    expect(parsed.audience).toEqual({ kind: "gm_only" });
  });
});

describe("PathfindSchema", () => {
  it("the Pathfind schema accepts an absent token", () => {
    expect(() =>
      PathfindSchema.parse({
        type: "pathfind",
        request_id: "r",
        scene: "s",
        start: [0, 0],
        waypoints: [[1, 1]],
        footprint_radius: 0.4,
      }),
    ).not.toThrow();
  });

  it("parses a named token", () => {
    const parsed = PathfindSchema.parse({
      type: "pathfind",
      request_id: "r",
      scene: "s",
      start: [0, 0],
      waypoints: [[1, 1]],
      footprint_radius: 0.4,
      token: "00000000-0000-0000-0000-000000000001",
    });
    expect(parsed.token).toBe("00000000-0000-0000-0000-000000000001");
  });

  it("rejects a non-uuid token", () => {
    expect(
      PathfindSchema.safeParse({
        type: "pathfind",
        request_id: "r",
        scene: "s",
        start: [0, 0],
        waypoints: [[1, 1]],
        footprint_radius: 0.4,
        token: "not-a-uuid",
      }).success,
    ).toBe(false);
  });
});
