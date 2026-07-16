import { describe, it, expect } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import SystemTreeEditor from "./SystemTreeEditor.svelte";
import { setAppContextForTest } from "./__fixtures__/appContextTest";
import type { WireDocument } from "@shadowcat/core";

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

  it("removeField dispatches the CURRENT container value read fresh as old", async () => {
    const calls: unknown[] = [];
    const context = setAppContextForTest({ dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const d = doc({ a: "1", b: "2" });
    const { getAllByRole } = render(SystemTreeEditor, { props: { doc: d, basePath: "/system", root: d.system, readOnly: false }, context });
    const removeButtons = getAllByRole("button", { name: "sheets.tree.remove" });
    await fireEvent.click(removeButtons[0]);
    expect(calls).toEqual([
      [{ op: "update", doc_id: "d1", changes: [{ path: "/system", old: { a: "1", b: "2" }, new: { b: "2" } }] }],
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
});
