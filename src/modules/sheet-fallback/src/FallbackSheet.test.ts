import { describe, it, expect, vi } from "vitest";
import { tick } from "svelte";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore, envelope } from "@shadowcat/core";
import FallbackSheet from "./FallbackSheet.svelte";

function storeWith(doc: ReturnType<typeof envelope>) {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc }] });
  return s;
}

describe("FallbackSheet reactive doc", () => {
  // `doc` must re-derive after a confirmed store update, or a second edit dispatches a
  // stale OCC `old` (silently rejected by the server) instead of the value the first
  // edit actually produced.
  it("re-derives doc after a store update so a second edit's OCC old reflects the first", async () => {
    const dispatchIntent = vi.fn();
    const doc = envelope("w1", "widget", null, { foo: "bar" }, "d1");
    const documents = storeWith(doc);
    render(FallbackSheet, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents, dispatchIntent, canEdit: () => true }),
      props: { docId: "d1", systemPrefix: "/system", close: () => {} },
    });

    const input = screen.getByLabelText("foo") as HTMLInputElement;
    await fireEvent.change(input, { target: { value: "baz" } });
    expect(dispatchIntent).toHaveBeenLastCalledWith([
      { op: "update", doc_id: "d1", changes: [{ path: "/system/foo", old: "bar", new: "baz" }] },
    ]);

    // Simulate the server confirming the first edit (a real broadcast landing on the store).
    documents.applyCommand({
      seq: 2,
      world_id: "w1",
      author: "a",
      ts: 0,
      ops: [{ op: "update", doc_id: "d1", changes: [{ path: "/system/foo", old: "bar", new: "baz" }] }],
    });
    await tick();

    const input2 = screen.getByLabelText("foo") as HTMLInputElement;
    await fireEvent.change(input2, { target: { value: "qux" } });
    // Asserts old reflects the confirmed first edit — a doc that fails to re-derive would
    // still read the stale "bar" here, silently rejected by the server's OCC check.
    expect(dispatchIntent).toHaveBeenLastCalledWith([
      { op: "update", doc_id: "d1", changes: [{ path: "/system/foo", old: "baz", new: "qux" }] },
    ]);
  });
});
