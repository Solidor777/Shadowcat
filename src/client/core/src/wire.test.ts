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
  type ServerMsg,
  type ClientMsg,
  type WireOperation,
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
    expectTypeOf<W["actor_role"]>().toEqualTypeOf<T["actor_role"]>();
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
});

describe("parseServerMsg", () => {
  it("validates a well-formed frame", () => {
    const m = parseServerMsg(
      JSON.stringify({
        type: "welcome",
        world: "w",
        current_seq: 0,
        server_time: 1,
        world_default_grants: { by_role: {}, by_user: {} },
        actor_role: "player",
        capability_requirements: [],
        contract_declarations: [],
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
        world_default_grants: { by_role: { owner: ["core:manage_embedded"] }, by_user: {} },
        actor_role: "gm",
        capability_requirements: [{ path_prefix: "/system/vision", caps: ["dnd5e:gm_vision"] }],
        contract_declarations: [],
      }),
    );
    expect(m?.type).toBe("welcome");
    if (m?.type === "welcome") {
      expect(m.actor_role).toBe("gm");
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

  it("returns null on malformed or unknown frames", () => {
    expect(parseServerMsg("{not json")).toBeNull();
    expect(parseServerMsg(JSON.stringify({ type: "nope" }))).toBeNull();
    expect(parseServerMsg(JSON.stringify({ type: "welcome" }))).toBeNull();
  });
});
