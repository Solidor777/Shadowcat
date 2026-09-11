import { describe, it, expect, vi } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import { buildTableDoc, type TableEngine, type WireSearchHit } from "@shadowcat/core";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import EntryEditor from "./EntryEditor.svelte";

function engine(): TableEngine {
  return { draw: { kind: "weighted" }, rows: [], description: "" };
}

describe("EntryEditor disabled", () => {
  it("disables every doc-hit button when disabled is true", async () => {
    const hitDoc = buildTableDoc("w1", "Rusty Key Table", engine(), { id: "hit1" });
    const searchDocuments = vi.fn((_q: string, _opts: unknown, onUpdate: (hits: WireSearchHit[]) => void) => {
      onUpdate([{ document: hitDoc, score: 1, snippet: "" }]);
      return Promise.resolve({ unsubscribe: () => {} });
    });
    const context = setAppContextForTest({ searchDocuments });
    const { getByTestId, getAllByRole } = render(EntryEditor, {
      props: {
        entry: { kind: "doc", target: { kind: "doc", doc_id: "", embedded_path: null }, label: "" },
        onChange: () => {},
        onRemove: () => {},
        disabled: true,
      },
      context,
    });
    await fireEvent.input(getByTestId("entry-pick-doc"), { target: { value: "rusty" } });
    const hitButtons = getAllByRole("button").filter((b) => b.textContent === "Rusty Key Table");
    expect(hitButtons.length).toBeGreaterThan(0);
    for (const b of hitButtons) {
      expect((b as HTMLButtonElement).disabled).toBe(true);
    }
  });

  it("disables every table-hit button when disabled is true", async () => {
    const hitDoc = buildTableDoc("w1", "Loot Table", engine(), { id: "hit2" });
    const searchDocuments = vi.fn((_q: string, _opts: unknown, onUpdate: (hits: WireSearchHit[]) => void) => {
      onUpdate([{ document: hitDoc, score: 1, snippet: "" }]);
      return Promise.resolve({ unsubscribe: () => {} });
    });
    const context = setAppContextForTest({ searchDocuments });
    const { getByTestId, getAllByRole } = render(EntryEditor, {
      props: {
        entry: { kind: "draw", table_id: "", count: 1 },
        onChange: () => {},
        onRemove: () => {},
        disabled: true,
      },
      context,
    });
    await fireEvent.input(getByTestId("entry-pick-table"), { target: { value: "loot" } });
    const hitButtons = getAllByRole("button").filter((b) => b.textContent === "Loot Table");
    expect(hitButtons.length).toBeGreaterThan(0);
    for (const b of hitButtons) {
      expect((b as HTMLButtonElement).disabled).toBe(true);
    }
  });

  it("disables the kind select, entry-kind-specific controls, and the remove button", () => {
    const context = setAppContextForTest({});
    const { getByTestId } = render(EntryEditor, {
      props: {
        entry: { kind: "text", text: "" },
        onChange: () => {},
        onRemove: () => {},
        disabled: true,
      },
      context,
    });
    expect((getByTestId("entry-kind") as HTMLSelectElement).disabled).toBe(true);
    expect((getByTestId("entry-text") as HTMLTextAreaElement).disabled).toBe(true);
    expect((getByTestId("entry-remove") as HTMLButtonElement).disabled).toBe(true);
  });
});
