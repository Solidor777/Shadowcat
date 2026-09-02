import { describe, it, expect, vi } from "vitest";
import { TemplatesController } from "./templatesController.svelte";
import {
  DocumentStore, silentLogger, MergeIntentError,
  type WireDocument, type WireOperation, type ClientMsg, type WireMergeOutcome,
} from "@shadowcat/core";

type MergeIntentMsg = Extract<ClientMsg, { type: "merge_pull" | "merge_push" | "merge_revert" }>;

function doc(over: Partial<WireDocument> & { id: string }): WireDocument {
  return {
    id: over.id, scope: { kind: "world", world_id: "w1" }, doc_type: over.doc_type ?? "actor",
    schema_version: 1, name: over.name ?? null, source: over.source ?? null, owner: over.owner ?? null,
    permissions: over.permissions ?? { default: "owner", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
    embedded: over.embedded ?? {}, parent_id: null, engine: over.engine, system: over.system ?? {},
    created_at: 0, updated_at: 0,
  };
}

/** Builds a `store`/`ctrl` pair; `sendMergeIntent` is a fake socket answering whatever
 * `answers` returns for each sent frame — the server-side merge computation is out of scope
 * for this controller's own tests (see `crate::merge`'s conformance corpus for that). */
function make(
  docs: WireDocument[],
  answers: (msg: MergeIntentMsg) => Promise<WireMergeOutcome>,
  over: Partial<{ role: "gm" | "player"; selfId: string; canEdit: (doc: WireDocument, path: string) => boolean }> = {},
) {
  const store = new DocumentStore();
  store.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((d) => ({ op: "create", doc: d } as WireOperation)) });
  const sent: MergeIntentMsg[] = [];
  const sendMergeIntent = vi.fn((msg: MergeIntentMsg) => {
    sent.push(msg);
    return answers(msg);
  });
  const ctrl = new TemplatesController({
    store, documents: store, sendMergeIntent,
    role: over.role ?? "gm", selfId: over.selfId ?? "u-self",
    canEdit: over.canEdit ?? (() => true), logger: silentLogger, notify: () => {},
  });
  return { store, ctrl, sent };
}

describe("TemplatesController", () => {
  it("pull with a conflict-free outcome sends merge_pull and leaves pending null", async () => {
    const tmpl = doc({ id: "T" });
    const child = doc({ id: "C", source: { id: "T", pack: null, version: 1 } });
    const { ctrl, sent } = make([tmpl, child], async () => ({ kind: "pull", child_id: "C", status: "applied" }));
    ctrl.pull("C");
    await vi.waitFor(() => expect(sent).toHaveLength(1));
    expect(sent[0].type).toBe("merge_pull");
    expect((sent[0] as { child_id: string }).child_id).toBe("C");
    expect((sent[0] as { resolutions?: unknown }).resolutions).toBeUndefined();
    expect(ctrl.pending).toBeNull();
  });

  it("pull with a conflicted outcome opens the modal, then resolve re-sends with resolutions", async () => {
    const tmpl = doc({ id: "T" });
    const child = doc({ id: "C", source: { id: "T", pack: null, version: 1 } });
    const { ctrl, sent } = make([tmpl, child], async (msg) => {
      if (!("resolutions" in msg) || !msg.resolutions) {
        return {
          kind: "pull", child_id: "C",
          status: { conflicts: [{ path: "/system/hp", base: 1, parent: 5, child: 9, parentKind: "set" }] },
        };
      }
      return { kind: "pull", child_id: "C", status: "applied" };
    });
    ctrl.pull("C");
    await vi.waitFor(() => expect(ctrl.pending).not.toBeNull());
    expect(sent).toHaveLength(1);
    expect(ctrl.pending!.groups[0].conflicts[0].path).toBe("/system/hp");
    ctrl.pending!.resolve(new Map([["C", new Set(["/system/hp"])]]));
    expect(ctrl.pending).toBeNull(); // closes eagerly on submit
    await vi.waitFor(() => expect(sent).toHaveLength(2));
    expect((sent[1] as { resolutions?: string[] }).resolutions).toEqual(["/system/hp"]);
  });

  it("pull reopens the modal with the fresh conflict set on StaleResolutions", async () => {
    const tmpl = doc({ id: "T" });
    const child = doc({ id: "C", source: { id: "T", pack: null, version: 1 } });
    const freshOutcome: WireMergeOutcome = {
      kind: "pull", child_id: "C",
      status: { conflicts: [{ path: "/system/mp", base: 1, parent: 2, child: 3, parentKind: "set" }] },
    };
    const { ctrl, sent } = make([tmpl, child], async (msg) => {
      if (!("resolutions" in msg) || !msg.resolutions) {
        return { kind: "pull", child_id: "C", status: { conflicts: [] } };
      }
      throw new MergeIntentError({ stale_resolutions: freshOutcome });
    });
    ctrl.pull("C");
    await vi.waitFor(() => expect(sent).toHaveLength(1));
    ctrl.pending!.resolve(new Map());
    await vi.waitFor(() => expect(ctrl.pending).not.toBeNull());
    expect(ctrl.pending!.groups[0].conflicts[0].path).toBe("/system/mp");
  });

  it("pull is a no-op with a logged warning when the child is not in store", () => {
    const warned: string[] = [];
    const sendMergeIntent = vi.fn(() => Promise.resolve({ kind: "revert", child_id: "x", status: "applied" } as WireMergeOutcome));
    const store = new DocumentStore();
    const ctrl = new TemplatesController({
      store, documents: store, sendMergeIntent,
      role: "gm", selfId: "u-self", canEdit: () => true,
      logger: { ...silentLogger, warn: (m: string) => warned.push(m) },
      notify: () => {},
    });
    ctrl.pull("ABSENT");
    expect(warned).toHaveLength(1);
    expect(warned[0]).toContain("templates.pull");
    expect(warned[0]).toContain("ABSENT");
    expect(sendMergeIntent).not.toHaveBeenCalled();
  });

  it("revert is a no-op with a logged warning when the child is not in store", () => {
    const warned: string[] = [];
    const store = new DocumentStore();
    const ctrl = new TemplatesController({
      store, documents: store, sendMergeIntent: () => Promise.resolve({ kind: "revert", child_id: "x", status: "applied" }),
      role: "gm", selfId: "u-self", canEdit: () => true,
      logger: { ...silentLogger, warn: (m: string) => warned.push(m) },
      notify: () => {},
    });
    ctrl.revert("ABSENT");
    expect(warned).toHaveLength(1);
    expect(warned[0]).toContain("templates.revert");
    expect(warned[0]).toContain("ABSENT");
  });

  it("revert sends merge_revert for a resolvable child", async () => {
    const tmpl = doc({ id: "T" });
    const child = doc({ id: "C", source: { id: "T", pack: null, version: 1 } });
    const { ctrl, sent } = make([tmpl, child], async () => ({ kind: "revert", child_id: "C", status: "applied" }));
    ctrl.revert("C");
    await vi.waitFor(() => expect(sent).toHaveLength(1));
    expect(sent[0].type).toBe("merge_revert");
  });

  it("push routes applied/conflicted/excluded instances from one outcome", async () => {
    const tmpl = doc({ id: "T" });
    const { ctrl, sent } = make([tmpl], async () => ({
      kind: "push",
      template_id: "T",
      instances: [
        { instance_id: "A", name: "Goblin A", status: "applied" },
        { instance_id: "B", name: "Goblin B", status: { conflicts: [{ path: "/system/hp", base: 1, parent: 5, child: 9, parentKind: "set" }] } },
        { instance_id: "C", name: "Goblin C", status: "excluded" },
      ],
    }), { role: "gm" });
    // `push()` itself only checks that the template resolves locally; the authoritative
    // instance set + per-instance authorization is entirely server-side (`instances_of`).
    ctrl.push("T");
    await vi.waitFor(() => expect(sent).toHaveLength(1));
    expect(sent[0].type).toBe("merge_push");
    expect(ctrl.pending).not.toBeNull();
    expect(ctrl.pending!.groups.map((g) => g.key)).toEqual(["B"]);
  });

  it("push's excluded-only outcome warns once (player-presentable, no raw instance id) and leaves pending null", async () => {
    const tmpl = doc({ id: "T" });
    const notified: { message: string; level?: string }[] = [];
    const store = new DocumentStore();
    store.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: tmpl }] });
    const ctrl = new TemplatesController({
      store, documents: store,
      sendMergeIntent: async () => ({ kind: "push", template_id: "T", instances: [{ instance_id: "B", name: null, status: "excluded" }] }),
      role: "gm", selfId: "u-self", canEdit: () => true, logger: silentLogger,
      notify: (message, level) => notified.push({ message, level }),
    });
    ctrl.push("T");
    await vi.waitFor(() => expect(notified).toHaveLength(1));
    expect(notified[0].level).toBe("warning");
    expect(notified[0].message).not.toContain("B");
    expect(ctrl.pending).toBeNull();
  });

  it("push's resolve re-sends merge_push keyed by instance id", async () => {
    const tmpl = doc({ id: "T" });
    const { ctrl, sent } = make([tmpl], async (msg) => {
      if (!("resolutions" in msg) || !msg.resolutions) {
        return {
          kind: "push", template_id: "T",
          instances: [{ instance_id: "B", name: "Goblin B", status: { conflicts: [{ path: "/system/hp", base: 1, parent: 5, child: 9, parentKind: "set" }] } }],
        };
      }
      return { kind: "push", template_id: "T", instances: [{ instance_id: "B", name: "Goblin B", status: "applied" }] };
    });
    ctrl.push("T");
    await vi.waitFor(() => expect(ctrl.pending).not.toBeNull());
    ctrl.pending!.resolve(new Map([["B", new Set(["/system/hp"])]]));
    await vi.waitFor(() => expect(sent).toHaveLength(2));
    expect((sent[1] as { resolutions?: Record<string, string[]> }).resolutions).toEqual({ B: ["/system/hp"] });
  });

  it("canPull is false for a non-owner non-GM", () => {
    const tmpl = doc({ id: "T" });
    const child = doc({ id: "C", owner: "someone-else", source: { id: "T", pack: null, version: 1 } });
    const { ctrl } = make([tmpl, child], async () => ({ kind: "revert", child_id: "C", status: "applied" }), { role: "player", selfId: "u-self" });
    expect(ctrl.canPull("C")).toBe(false);
  });

  it("canPull is false for a user who can edit system but not embedded", () => {
    const tmpl = doc({ id: "T" });
    const child = doc({ id: "C", owner: "u-self", source: { id: "T", pack: null, version: 1 } });
    const { ctrl } = make([tmpl, child], async () => ({ kind: "revert", child_id: "C", status: "applied" }), {
      role: "player", selfId: "u-self", canEdit: (_doc, path) => path === "/system",
    });
    expect(ctrl.canPull("C")).toBe(false);
  });

  it("canPull is true for a user who can edit system + embedded even without /base (the /base leg is dropped)", () => {
    const tmpl = doc({ id: "T" });
    const child = doc({ id: "C", owner: "u-self", source: { id: "T", pack: null, version: 1 } });
    const { ctrl } = make([tmpl, child], async () => ({ kind: "revert", child_id: "C", status: "applied" }), {
      role: "player", selfId: "u-self", canEdit: (_doc, path) => path === "/system" || path === "/embedded",
    });
    expect(ctrl.canPull("C")).toBe(true);
  });

  it("canPush is false for a user who can edit system but not embedded", () => {
    const tmpl = doc({ id: "T", owner: "u-self" });
    const inst = doc({ id: "A", source: { id: "T", pack: null, version: 1 } });
    const { ctrl } = make([tmpl, inst], async () => ({ kind: "revert", child_id: "A", status: "applied" }), {
      role: "player", selfId: "u-self", canEdit: (_doc, path) => path === "/system",
    });
    expect(ctrl.canPush("T")).toBe(false);
  });

  it("findInstances returns instances of the template from the store", () => {
    const tmpl = doc({ id: "T" });
    const a = doc({ id: "A", source: { id: "T", pack: null, version: 1 } });
    const { ctrl } = make([tmpl, a], async () => ({ kind: "revert", child_id: "A", status: "applied" }));
    expect(ctrl.findInstances("T").map((d) => d.id)).toEqual(["A"]);
  });

  it("treats the inheriting owner of a linked token as owner for template controls", () => {
    // token instance: owner null, engine.actor_id -> actor owned by self.
    // Literal doc.owner gate hid pull; the effectiveOwner mirror must show it.
    const tmpl = doc({ id: "T", doc_type: "actor" });
    const actor = doc({ id: "ACT", doc_type: "actor", owner: "u-self" });
    const token = doc({
      id: "TOK", doc_type: "token", owner: null,
      engine: { actor_id: "ACT" }, source: { id: "T", pack: null, version: 1 },
    });
    const { ctrl } = make([tmpl, actor, token], async () => ({ kind: "revert", child_id: "TOK", status: "applied" }), {
      role: "player", selfId: "u-self",
    });
    expect(ctrl.canPull("TOK")).toBe(true);
  });
});
