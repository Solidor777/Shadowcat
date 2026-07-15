import { describe, it, expect } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import SystemTreeEditor from "./SystemTreeEditor.svelte";
import { setAppContextForTest } from "./__fixtures__/appContextTest";
import type { WireDocument } from "@shadowcat/core";

function doc(system: unknown): WireDocument {
  return { id: "d1", scope: { kind: "world", world_id: "w1" }, doc_type: "actor", schema_version: 1, source: null, owner: null, permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null }, embedded: {}, parent_id: null, system, created_at: 0, updated_at: 0 };
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
});
