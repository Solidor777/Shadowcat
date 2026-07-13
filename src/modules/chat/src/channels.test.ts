import { describe, expect, test } from "vitest";
import type { ChatMessageSystem, WireDocument } from "@shadowcat/core";
import { postTarget, inView, byCreation, RENDER_CAP, type ChatView } from "./channels";

function sys(over: Partial<ChatMessageSystem> = {}): ChatMessageSystem {
  return {
    channel: "general",
    user_owner: "u1",
    kind: "normal",
    audience: { kind: "public" },
    content: [{ kind: "text", text: "hi" }],
    ...over,
  };
}

const AUDIENCES: ChatMessageSystem["audience"][] = [
  { kind: "public" },
  { kind: "whisper", recipients: ["u2"] },
  { kind: "gm_only" },
];

describe("postTarget", () => {
  test("all view posts to the default channel with public audience", () => {
    expect(postTarget({ kind: "all" })).toEqual({ channel: "general", audience: { kind: "public" } });
  });
  test("channel view posts to that channel with public audience", () => {
    expect(postTarget({ kind: "channel", id: "ooc" })).toEqual({ channel: "ooc", audience: { kind: "public" } });
  });
  test("gm view posts to the default channel with gm_only audience", () => {
    expect(postTarget({ kind: "gm" })).toEqual({ channel: "general", audience: { kind: "gm_only" } });
  });
});

describe("inView", () => {
  const views: ChatView[] = [{ kind: "all" }, { kind: "channel", id: "general" }, { kind: "gm" }];

  for (const view of views) {
    for (const audience of AUDIENCES) {
      test(`view=${view.kind} audience=${audience.kind}: channel match`, () => {
        const s = sys({ channel: "general", audience });
        const expected = view.kind === "all" ? true : view.kind === "gm" ? audience.kind === "gm_only" : true;
        expect(inView(view, s)).toBe(expected);
      });
    }
  }

  test("channel view: mismatched channel is excluded regardless of audience", () => {
    for (const audience of AUDIENCES) {
      expect(inView({ kind: "channel", id: "ooc" }, sys({ channel: "general", audience }))).toBe(false);
    }
  });

  test("channel view: matching channel is included regardless of audience", () => {
    for (const audience of AUDIENCES) {
      expect(inView({ kind: "channel", id: "general" }, sys({ channel: "general", audience }))).toBe(true);
    }
  });

  test("gm view: only gm_only audience passes, public/whisper excluded, channel irrelevant", () => {
    expect(inView({ kind: "gm" }, sys({ channel: "ooc", audience: { kind: "gm_only" } }))).toBe(true);
    expect(inView({ kind: "gm" }, sys({ channel: "general", audience: { kind: "public" } }))).toBe(false);
    expect(inView({ kind: "gm" }, sys({ channel: "general", audience: { kind: "whisper", recipients: [] } }))).toBe(false);
  });

  test("all view: everything passes regardless of channel/audience", () => {
    for (const audience of AUDIENCES) {
      expect(inView({ kind: "all" }, sys({ channel: "anything", audience }))).toBe(true);
    }
  });
});

function doc(id: string, created_at: number): WireDocument {
  return {
    id,
    scope: { kind: "world", world_id: "w1" },
    doc_type: "message",
    schema_version: 1,
    source: null,
    owner: "u1",
    permissions: { default: "observer", users: {} } as WireDocument["permissions"],
    embedded: {},
    parent_id: null,
    system: {},
    created_at,
    updated_at: created_at,
  };
}

describe("byCreation", () => {
  test("sorts ascending by created_at", () => {
    const a = doc("a", 5);
    const b = doc("b", 1);
    expect([a, b].sort(byCreation)).toEqual([b, a]);
  });

  test("ties on created_at break by id ascending", () => {
    const a = doc("b", 1);
    const b = doc("a", 1);
    expect([a, b].sort(byCreation)).toEqual([b, a]);
  });

  test("equal created_at and id sorts to 0 (stable)", () => {
    const a = doc("a", 1);
    const b = doc("a", 1);
    expect(byCreation(a, b)).toBe(0);
  });
});

test("RENDER_CAP is 200", () => {
  expect(RENDER_CAP).toBe(200);
});
