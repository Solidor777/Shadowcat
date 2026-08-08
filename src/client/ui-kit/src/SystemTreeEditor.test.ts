import { describe, it, expect } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import SystemTreeEditor from "./SystemTreeEditor.svelte";
import systemTreeEditorSource from "./SystemTreeEditor.svelte?raw";
import { setAppContextForTest } from "./__fixtures__/appContextTest";
import { DocumentStore, type WireDocument, type WireCommand } from "@shadowcat/core";

function doc(system: unknown): WireDocument {
  return { id: "d1", scope: { kind: "world", world_id: "w1" }, doc_type: "actor", schema_version: 1, name: null, source: null, owner: null, permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null }, embedded: {}, parent_id: null, system, created_at: 0, updated_at: 0 };
}

describe("SystemTreeEditor", () => {
  it("edits a string leaf dispatching the real pre-image", async () => {
    const calls: unknown[] = [];
    const context = setAppContextForTest({ dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const d = doc({ name: "Goblin" });
    const { getByDisplayValue } = render(SystemTreeEditor, { props: { doc: d, basePath: "/system", root: d.system, readOnly: false }, context });
    const input = getByDisplayValue("Goblin");
    await fireEvent.change(input, { target: { value: "Orc" } });
    expect(calls).toEqual([[{ op: "update", doc_id: "d1", changes: [{ path: "/system/name", old: "Goblin", new: "Orc" }] }]]);
  });

  it("renders read-only inputs when readOnly is set (no dispatch)", async () => {
    const calls: unknown[] = [];
    const context = setAppContextForTest({ dispatchIntent: (ops) => calls.push(ops) });
    const d = doc({ name: "Goblin" });
    const { getByDisplayValue } = render(SystemTreeEditor, { props: { doc: d, basePath: "/system", root: d.system, readOnly: true }, context });
    expect((getByDisplayValue("Goblin") as HTMLInputElement).disabled).toBe(true);
  });

  it("removeField on an object key dispatches a narrow-OCC leaf removal (remove: true)", async () => {
    const calls: unknown[] = [];
    const context = setAppContextForTest({ dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const d = doc({ a: "1", b: "2" });
    const { getAllByRole } = render(SystemTreeEditor, { props: { doc: d, basePath: "/system", root: d.system, readOnly: false }, context });
    const removeButtons = getAllByRole("button", { name: "sheets.tree.remove" });
    await fireEvent.click(removeButtons[0]);
    // Leaf remove of the key `a` with only that key's pre-image as `old` — NOT a whole-
    // container replacement — so a concurrent sibling edit to `b` does not conflict.
    expect(calls).toEqual([
      [{ op: "update", doc_id: "d1", changes: [{ path: "/system/a", old: "1", new: null, remove: true }] }],
    ]);
  });

  it("removeField round-trips through the real store: the key is genuinely absent", async () => {
    // End-to-end guard (not just the dispatched wire message): the emitted op, applied by the
    // real DocumentStore, must DELETE the key — proving `DocumentStore` honors `remove: true`, the
    // client-visible half of the null != absent guarantee.
    const ops: WireCommand["ops"][] = [];
    const context = setAppContextForTest({ dispatchIntent: (o) => ops.push(o), canEdit: () => true });
    const d = doc({ a: "1", b: "2" });
    const store = new DocumentStore();
    store.applyCommand({ seq: 1, world_id: "w1", author: "t", ts: 0, ops: [{ op: "create", doc: d }] });
    const { getAllByRole } = render(SystemTreeEditor, { props: { doc: d, basePath: "/system", root: d.system, readOnly: false }, context });
    await fireEvent.click(getAllByRole("button", { name: "sheets.tree.remove" })[0]);
    store.applyCommand({ seq: 2, world_id: "w1", author: "t", ts: 0, ops: ops[0] });
    const system = store.get("d1")!.system as Record<string, unknown>;
    expect("a" in system).toBe(false); // genuinely absent, not null
    expect(system.b).toBe("2");
  });

  it("removeField on an array element stays whole-array replacement (no remove flag)", async () => {
    const calls: unknown[] = [];
    const context = setAppContextForTest({ dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const d = doc({ arr: ["x", "y", "z"] });
    const { getAllByRole } = render(SystemTreeEditor, { props: { doc: d, basePath: "/system/arr", root: (d.system as { arr: unknown[] }).arr, readOnly: false }, context });
    const removeButtons = getAllByRole("button", { name: "sheets.tree.remove" });
    await fireEvent.click(removeButtons[1]);
    expect(calls).toEqual([
      [{ op: "update", doc_id: "d1", changes: [{ path: "/system/arr", old: ["x", "y", "z"], new: ["x", "z"] }] }],
    ]);
  });

  it("addArrayItem on a number[] seeds the new element as 0", async () => {
    const calls: unknown[] = [];
    const context = setAppContextForTest({ dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const d = doc({ arr: [1, 2, 3] });
    const { getByText } = render(SystemTreeEditor, { props: { doc: d, basePath: "/system/arr", root: (d.system as { arr: unknown[] }).arr, readOnly: false }, context });
    await fireEvent.click(getByText("sheets.tree.addItem"));
    expect(calls).toEqual([
      [{ op: "update", doc_id: "d1", changes: [{ path: "/system/arr", old: [1, 2, 3], new: [1, 2, 3, 0] }] }],
    ]);
  });

  it("addArrayItem on a boolean[] seeds the new element as false", async () => {
    const calls: unknown[] = [];
    const context = setAppContextForTest({ dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const d = doc({ arr: [true, false] });
    const { getByText } = render(SystemTreeEditor, { props: { doc: d, basePath: "/system/arr", root: (d.system as { arr: unknown[] }).arr, readOnly: false }, context });
    await fireEvent.click(getByText("sheets.tree.addItem"));
    expect(calls).toEqual([
      [{ op: "update", doc_id: "d1", changes: [{ path: "/system/arr", old: [true, false], new: [true, false, false] }] }],
    ]);
  });

  it("addArrayItem on an empty array seeds the new element as an empty string", async () => {
    const calls: unknown[] = [];
    const context = setAppContextForTest({ dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const d = doc({ arr: [] });
    const { getByText } = render(SystemTreeEditor, { props: { doc: d, basePath: "/system/arr", root: (d.system as { arr: unknown[] }).arr, readOnly: false }, context });
    await fireEvent.click(getByText("sheets.tree.addItem"));
    expect(calls).toEqual([
      [{ op: "update", doc_id: "d1", changes: [{ path: "/system/arr", old: [], new: [""] }] }],
    ]);
  });

  it("addField on an object dispatches old: null for the new key's own path", async () => {
    const calls: unknown[] = [];
    const context = setAppContextForTest({ dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const d = doc({ a: "1" });
    const { getByText } = render(SystemTreeEditor, { props: { doc: d, basePath: "/system", root: d.system, readOnly: false }, context });
    await fireEvent.click(getByText("sheets.tree.addField"));
    expect(calls.length).toBe(1);
    const change = (calls[0] as { changes: { path: string; old: unknown; new: unknown }[] }[])[0].changes[0];
    expect(change.path).toMatch(/^\/system\/[0-9a-f-]{8}$/);
    expect(change.old).toBeNull();
    expect(change.new).toBe("");
  });

  it("recursion: editing a leaf at depth 2 dispatches against its own full path and pre-image", async () => {
    const calls: unknown[] = [];
    const context = setAppContextForTest({ dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const d = doc({ a: { b: "x" } });
    const { getByDisplayValue } = render(SystemTreeEditor, { props: { doc: d, basePath: "/system", root: d.system, readOnly: false }, context });
    const input = getByDisplayValue("x");
    await fireEvent.change(input, { target: { value: "y" } });
    expect(calls).toEqual([[{ op: "update", doc_id: "d1", changes: [{ path: "/system/a/b", old: "x", new: "y" }] }]]);
  });

  it("the text/number/checkbox leaf inputs carry the shared coarse-pointer touch-sizing rule", () => {
    // jsdom doesn't evaluate @media (pointer: coarse), so assert the rule's
    // presence directly in the component's source styles instead (mirrors the "select/input
    // controls get a 44px coarse-pointer min-height" test).
    const ruleMatch = systemTreeEditorSource.match(/\.node input\s*\{([^}]*@media[^}]*\{[^}]*\}[^}]*)\}/);
    expect(ruleMatch).toBeTruthy();
    expect(ruleMatch?.[1]).toMatch(/@media \(pointer: coarse\)\s*\{\s*min-height:\s*var\(--input-height-coarse\);\s*\}/);
  });
});
