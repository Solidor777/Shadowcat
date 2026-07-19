import { describe, it, expect } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import MergeConflictModal from "./MergeConflictModal.svelte";
import { setAppContextForTest } from "./__fixtures__/appContextTest";
import type { Conflict } from "@shadowcat/core";

const conflicts: Conflict[] = [
  { path: "/system/hp", base: 1, parent: 5, child: 9, parentKind: "set" },
  { path: "/system/name", base: "x", parent: "y", child: "z", parentKind: "set" },
];

describe("MergeConflictModal", () => {
  it("renders one row per conflict with base/template/mine values", () => {
    const context = setAppContextForTest();
    const { getByText } = render(MergeConflictModal, {
      props: { groups: [{ key: "C", label: null, conflicts }], onApply: () => {}, onCancel: () => {} },
      context,
    });
    expect(getByText("/system/hp")).toBeTruthy();
    expect(getByText("5")).toBeTruthy(); // template value
    expect(getByText("9")).toBeTruthy(); // mine value
  });

  it("Apply reports only the fields switched to 'take template'", async () => {
    const applied: Map<string, Set<string>>[] = [];
    const context = setAppContextForTest();
    const { getByText, getAllByRole } = render(MergeConflictModal, {
      props: { groups: [{ key: "C", label: null, conflicts }], onApply: (m) => applied.push(m), onCancel: () => {} },
      context,
    });
    // radios come in pairs (keep mine / take template) per row; switch row 0 to template.
    const radios = getAllByRole("radio") as HTMLInputElement[];
    const takeTemplateRow0 = radios.find((r) => r.value === "theirs" && r.name === "C /system/hp")!;
    await fireEvent.click(takeTemplateRow0);
    await fireEvent.click(getByText("templates.conflict.apply"));
    expect(applied).toHaveLength(1);
    expect([...applied[0].get("C")!]).toEqual(["/system/hp"]);
  });

  it("Cancel reports nothing applied", async () => {
    let cancelled = false;
    const context = setAppContextForTest();
    const { getByText } = render(MergeConflictModal, {
      props: { groups: [{ key: "C", label: null, conflicts }], onApply: () => {}, onCancel: () => { cancelled = true; } },
      context,
    });
    await fireEvent.click(getByText("templates.conflict.cancel"));
    expect(cancelled).toBe(true);
  });

  it("groups rows under a per-instance label when provided (push)", () => {
    const context = setAppContextForTest();
    const { getByText } = render(MergeConflictModal, {
      props: {
        groups: [
          { key: "i1", label: "Goblin A", conflicts: [conflicts[0]] },
          { key: "i2", label: "Goblin B", conflicts: [conflicts[1]] },
        ],
        onApply: () => {}, onCancel: () => {},
      },
      context,
    });
    expect(getByText("Goblin A")).toBeTruthy();
    expect(getByText("Goblin B")).toBeTruthy();
  });
});
