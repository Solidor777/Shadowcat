import { describe, expect, test } from "vitest";
import type { ChatMessageEngine, WireDocument } from "@shadowcat/core";
import {
  postTarget,
  inView,
  byCreation,
  RENDER_CAP,
  createChatDerivationCache,
  deriveVisibleDocs,
  computeVisibleWindow,
  VIRTUALIZE_OVERSCAN,
  type ChatView,
} from "./channels";

function sys(over: Partial<ChatMessageEngine> = {}): ChatMessageEngine {
  return {
    channel: "general",
    user_owner: "u1",
    kind: "normal",
    audience: { kind: "public" },
    content: [{ kind: "text", text: "hi" }],
    ...over,
  };
}

const AUDIENCES: ChatMessageEngine["audience"][] = [
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
    name: null,
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

function engineDoc(id: string, created_at: number, engine: Record<string, unknown>): WireDocument {
  return {
    id,
    scope: { kind: "world", world_id: "w1" },
    doc_type: "message",
    schema_version: 1,
    name: null,
    source: null,
    owner: "u1",
    permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
    embedded: {},
    parent_id: null,
    engine,
    system: {},
    created_at,
    updated_at: created_at,
  } as WireDocument;
}
const publicEngineMsg = (id: string, created_at: number, text = `msg-${id}`) =>
  engineDoc(id, created_at, { channel: "general", user_owner: "u1", kind: "normal", audience: { kind: "public" }, content: [{ kind: "text", text }] });
const gmEngineMsg = (id: string, created_at: number) =>
  engineDoc(id, created_at, { channel: "general", user_owner: "u1", kind: "normal", audience: { kind: "gm_only" }, content: [{ kind: "text", text: `msg-${id}` }] });

describe("deriveVisibleDocs", () => {
  test("returns in-view messages sorted by creation, capped to the last N", () => {
    const cache = createChatDerivationCache();
    const docs = [publicEngineMsg("a", 3), publicEngineMsg("b", 1), publicEngineMsg("c", 2)];
    const result = deriveVisibleDocs(cache, docs, { kind: "all" }, 10);
    expect(result.map((d) => d.id)).toEqual(["b", "c", "a"]);
  });

  test("excludes messages that fail the view filter", () => {
    const cache = createChatDerivationCache();
    const docs = [publicEngineMsg("a", 1), gmEngineMsg("b", 2)];
    const result = deriveVisibleDocs(cache, docs, { kind: "gm" }, 10);
    expect(result.map((d) => d.id)).toEqual(["b"]);
  });

  test("caps to the last `cap` entries, dropping the oldest", () => {
    const cache = createChatDerivationCache();
    const docs = Array.from({ length: 5 }, (_, i) => publicEngineMsg(`m${i}`, i));
    const result = deriveVisibleDocs(cache, docs, { kind: "all" }, 3);
    expect(result.map((d) => d.id)).toEqual(["m2", "m3", "m4"]);
  });

  test("a subsequent call with a new (edited) reference for a known id refreshes content without reordering", () => {
    const cache = createChatDerivationCache();
    const docs = [publicEngineMsg("a", 1), publicEngineMsg("b", 2)];
    const first = deriveVisibleDocs(cache, docs, { kind: "all" }, 10);
    expect(first.map((d) => d.id)).toEqual(["a", "b"]);

    const editedA = publicEngineMsg("a", 1, "edited");
    const second = deriveVisibleDocs(cache, [editedA, docs[1]], { kind: "all" }, 10);
    expect(second.map((d) => d.id)).toEqual(["a", "b"]); // order unchanged
    expect((second[0].engine as { content: { text: string }[] }).content[0].text).toBe("edited");
  });

  test("a doc with an unchanged reference is skipped without re-deriving membership", () => {
    const cache = createChatDerivationCache();
    const docs = [publicEngineMsg("a", 1)];
    deriveVisibleDocs(cache, docs, { kind: "all" }, 10);
    // Same array, same object references: nothing new to process.
    const result = deriveVisibleDocs(cache, docs, { kind: "all" }, 10);
    expect(result.map((d) => d.id)).toEqual(["a"]);
  });

  test("evicts a non-member doc's cache entry when it disappears from allMessages", () => {
    const cache = createChatDerivationCache();
    // "b" fails the gm view filter (public audience), so it is cached in
    // `refs`/`members` but never enters `order`.
    const docs = [gmEngineMsg("a", 1), publicEngineMsg("b", 2)];
    deriveVisibleDocs(cache, docs, { kind: "gm" }, 10);
    expect(cache.refs.has("b")).toBe(true);
    expect(cache.members.has("b")).toBe(true);
    expect(cache.order).not.toContain("b");

    // "b" disappears from the store (shrink); only the member doc "a" remains.
    deriveVisibleDocs(cache, [docs[0]], { kind: "gm" }, 10);
    expect(cache.refs.has("b")).toBe(false);
    expect(cache.members.has("b")).toBe(false);
  });
});

describe("computeVisibleWindow", () => {
  test("returns the full range when the container has no measured layout (clientHeight <= 0)", () => {
    expect(computeVisibleWindow(0, 0, 0, 500)).toEqual({ start: 0, end: 500 });
  });

  test("returns the full range when content fits without overflowing the container", () => {
    expect(computeVisibleWindow(0, 400, 300, 500)).toEqual({ start: 0, end: 500 });
  });

  test("returns zero range for an empty list", () => {
    expect(computeVisibleWindow(0, 400, 5000, 0)).toEqual({ start: 0, end: 0 });
  });

  test("scrolled to the top windows near the start, padded by overscan", () => {
    const { start, end } = computeVisibleWindow(0, 400, 50000, 200);
    expect(start).toBe(0);
    expect(end).toBeLessThan(30);
  });

  test("scrolled to the bottom windows near the end, padded by overscan", () => {
    const { start, end } = computeVisibleWindow(50000 - 400, 400, 50000, 200);
    expect(end).toBe(200);
    expect(start).toBeGreaterThan(150);
  });

  test("a custom overscan widens the window on both sides", () => {
    const narrow = computeVisibleWindow(25000, 400, 50000, 200, 0);
    const wide = computeVisibleWindow(25000, 400, 50000, 200, 20);
    expect(wide.start).toBeLessThanOrEqual(narrow.start);
    expect(wide.end).toBeGreaterThanOrEqual(narrow.end);
    expect(wide.end - wide.start).toBeGreaterThan(narrow.end - narrow.start);
  });

  test("VIRTUALIZE_OVERSCAN is the default overscan", () => {
    expect(computeVisibleWindow(25000, 400, 50000, 200)).toEqual(computeVisibleWindow(25000, 400, 50000, 200, VIRTUALIZE_OVERSCAN));
  });
});
