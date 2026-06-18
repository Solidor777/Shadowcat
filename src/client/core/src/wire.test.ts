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
      }),
    );
    expect(m?.type).toBe("welcome");
  });

  it("returns null on malformed or unknown frames", () => {
    expect(parseServerMsg("{not json")).toBeNull();
    expect(parseServerMsg(JSON.stringify({ type: "nope" }))).toBeNull();
    expect(parseServerMsg(JSON.stringify({ type: "welcome" }))).toBeNull();
  });
});
