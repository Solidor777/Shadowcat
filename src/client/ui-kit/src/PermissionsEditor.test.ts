import { describe, it, expect } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import { DocumentStore, type WireDocument, type WireOperation } from "@shadowcat/core";
import PermissionsEditor from "./PermissionsEditor.svelte";
import UserMultiSelect from "./UserMultiSelect.svelte";
import { setAppContextForTest } from "./__fixtures__/appContextTest";

function doc(over: Partial<WireDocument["permissions"]> = {}): WireDocument {
  return {
    id: "d1", scope: { kind: "world", world_id: "w1" }, doc_type: "note", schema_version: 1,
    name: null, source: null, owner: null,
    permissions: { default: "none", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null, ...over },
    embedded: {}, parent_id: null, engine: undefined, system: {}, created_at: 0, updated_at: 0,
  };
}

function storeWith(d: WireDocument): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: d } as WireOperation] });
  return s;
}

const members = new Map([["u-a", "Ada"], ["u-b", "Bo"]]);

describe("PermissionsEditor", () => {
  it("edits the default role with the stored value as the pre-image", async () => {
    const calls: unknown[] = [];
    const documents = storeWith(doc());
    const context = setAppContextForTest({ documents, members, dispatchIntent: (ops) => calls.push(ops) });
    const { getByTestId } = render(PermissionsEditor, { props: { docId: "d1" }, context });
    await fireEvent.change(getByTestId("permissions-default"), { target: { value: "observer" } });
    expect(calls).toEqual([[{ op: "update", doc_id: "d1", changes: [{ path: "/permissions/default", old: "none", new: "observer" }] }]]);
  });

  it("adds a per-user override as an absent-key write, then edits its role", async () => {
    const calls: unknown[] = [];
    const documents = storeWith(doc());
    const context = setAppContextForTest({ documents, members, dispatchIntent: (ops) => calls.push(ops) });
    const { getAllByTestId } = render(PermissionsEditor, { props: { docId: "d1" }, context });
    await fireEvent.click(getAllByTestId("user-multi-select-option")[0]);
    expect(calls).toEqual([[{ op: "update", doc_id: "d1", changes: [{ path: "/permissions/users/u-a", old: null, new: "observer" }] }]]);
  });

  it("edits an existing override's role and removes it when unchecked", async () => {
    const calls: unknown[] = [];
    const documents = storeWith(doc({ users: { "u-b": "observer" } }));
    const context = setAppContextForTest({ documents, members, dispatchIntent: (ops) => calls.push(ops) });
    const { getByTestId, getAllByTestId } = render(PermissionsEditor, { props: { docId: "d1" }, context });
    await fireEvent.change(getByTestId("permissions-user-role"), { target: { value: "owner" } });
    await fireEvent.click(getAllByTestId("user-multi-select-option")[1]);
    expect(calls).toEqual([
      [{ op: "update", doc_id: "d1", changes: [{ path: "/permissions/users/u-b", old: "observer", new: "owner" }] }],
      [{ op: "update", doc_id: "d1", changes: [{ path: "/permissions/users/u-b", old: "observer", new: null, remove: true }] }],
    ]);
  });

  it("offers the GM cap only when enabled, and clears it with null", async () => {
    const calls: unknown[] = [];
    const documents = storeWith(doc({ gm_role: "observer" }));
    const context = setAppContextForTest({ documents, members, dispatchIntent: (ops) => calls.push(ops) });
    const off = render(PermissionsEditor, { props: { docId: "d1" }, context });
    expect(off.queryByTestId("permissions-gm-role")).toBeNull();
    off.unmount();
    const on = render(PermissionsEditor, { props: { docId: "d1", gmRole: true }, context });
    await fireEvent.change(on.getByTestId("permissions-gm-role"), { target: { value: "" } });
    expect(calls).toEqual([[{ op: "update", doc_id: "d1", changes: [{ path: "/permissions/gm_role", old: "observer", new: null }] }]]);
  });

  it("renders nothing for a viewer who may not write permissions", () => {
    const documents = storeWith(doc());
    const context = setAppContextForTest({ documents, members, canEdit: (_d, path) => path !== "/permissions" });
    const { queryByTestId } = render(PermissionsEditor, { props: { docId: "d1" }, context });
    expect(queryByTestId("permissions-editor")).toBeNull();
  });

  it("re-renders the override list after the store changes post-mount", async () => {
    const documents = storeWith(doc());
    const context = setAppContextForTest({ documents, members });
    const { queryAllByTestId } = render(PermissionsEditor, { props: { docId: "d1" }, context });
    expect(queryAllByTestId("permissions-user-role")).toHaveLength(0);
    documents.applyCommand({
      seq: 2, world_id: "w1", author: "a", ts: 0,
      ops: [{ op: "update", doc_id: "d1", changes: [{ path: "/permissions/users/u-a", old: null, new: "observer" }] }],
    });
    await Promise.resolve();
    expect(queryAllByTestId("permissions-user-role")).toHaveLength(1);
  });
});

describe("UserMultiSelect", () => {
  it("reports the next selection with the toggled id added or removed", async () => {
    const seen: string[][] = [];
    const { getAllByTestId } = render(UserMultiSelect, { props: { members, selected: ["u-b"], label: "Pick", onchange: (n: string[]) => seen.push(n) } });
    const boxes = getAllByTestId("user-multi-select-option") as HTMLInputElement[];
    expect(boxes.map((b) => b.checked)).toEqual([false, true]);
    await fireEvent.click(boxes[0]);
    await fireEvent.click(boxes[1]);
    expect(seen).toEqual([["u-b", "u-a"], []]);
  });
});
