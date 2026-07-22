import { test, expect } from "vitest";
import type { WireDocument } from "@shadowcat/core";
import { computeUnreadCount, markAllRead } from "./unread";

function msgDoc(id: string, created_at: number, engine: Record<string, unknown>): WireDocument {
  return {
    id,
    scope: { kind: "world", world_id: "w1" },
    doc_type: "message",
    schema_version: 1,
    name: null,
    source: null,
    owner: "u1",
    permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null } as WireDocument["permissions"],
    embedded: {},
    parent_id: null,
    engine,
    system: {},
    created_at,
    updated_at: created_at,
  };
}
const msg = (id: string, created_at: number, opts: { channel?: string; user_owner?: string } = {}) =>
  msgDoc(id, created_at, {
    channel: opts.channel ?? "general",
    user_owner: opts.user_owner ?? "u1",
    kind: "normal",
    audience: { kind: "public" },
    content: [{ kind: "text", text: `msg-${id}` }],
  });

test("computeUnreadCount counts every message with no read marker for its channel", () => {
  const count = computeUnreadCount([msg("m1", 1)], {}, "self");
  expect(count).toBe(1);
});

test("computeUnreadCount excludes messages authored by the reading user themself", () => {
  const count = computeUnreadCount([msg("m1", 1, { user_owner: "self" })], {}, "self");
  expect(count).toBe(0);
});

test("computeUnreadCount excludes messages at or before the channel's read marker", () => {
  const read = { general: { createdAt: 1, id: "m1" } };
  expect(computeUnreadCount([msg("m1", 1)], read, "self")).toBe(0);
  expect(computeUnreadCount([msg("m2", 2)], read, "self")).toBe(1);
});

test("computeUnreadCount tracks markers independently per channel", () => {
  const read = { general: { createdAt: 5, id: "m5" } };
  const docs = [msg("m1", 1, { channel: "general" }), msg("m2", 2, { channel: "ooc" })];
  expect(computeUnreadCount(docs, read, "self")).toBe(1); // only the unmarked "ooc" channel counts
});

test("computeUnreadCount tie-breaks equal timestamps by id, matching byCreation", () => {
  const read = { general: { createdAt: 1, id: "m1" } };
  expect(computeUnreadCount([msg("m0", 1)], read, "self")).toBe(0); // "m0" < "m1"
  expect(computeUnreadCount([msg("m2", 1)], read, "self")).toBe(1); // "m2" > "m1"
});

test("computeUnreadCount ignores non-message/unparseable docs", () => {
  const notAMessage: WireDocument = { ...msg("x", 1), doc_type: "scene" };
  expect(computeUnreadCount([notAMessage], {}, "self")).toBe(0);
});

test("markAllRead snapshots the newest message per channel", () => {
  const docs = [msg("m1", 1, { channel: "general" }), msg("m2", 2, { channel: "general" }), msg("m3", 1, { channel: "ooc" })];
  const read = markAllRead(docs);
  expect(read.general).toEqual({ createdAt: 2, id: "m2" });
  expect(read.ooc).toEqual({ createdAt: 1, id: "m3" });
});

test("markAllRead then computeUnreadCount over the same docs yields zero", () => {
  const docs = [msg("m1", 1), msg("m2", 2)];
  const read = markAllRead(docs);
  expect(computeUnreadCount(docs, read, "self")).toBe(0);
});

test("markAllRead on an empty list yields an empty read state", () => {
  expect(markAllRead([])).toEqual({});
});
