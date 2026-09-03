import { describe, it, expect, vi } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import VisionAssignmentsEditor from "./VisionAssignmentsEditor.svelte";
import { setAppContextForTest } from "./__fixtures__/appContextTest";
import { SEED_VISION_MODES, type VisionAssignment, type VisionMode } from "@shadowcat/core";

const modes: VisionMode[] = Object.values(SEED_VISION_MODES);
const darkvision: VisionAssignment = { mode: "darkvision", range: 12 };

describe("VisionAssignmentsEditor", () => {
  it("commits a whole-list update when a row's mode changes, preserving siblings", async () => {
    const onCommit = vi.fn();
    const rows: VisionAssignment[] = [darkvision, { mode: "tremorsense", range: null }];
    const { getByTestId } = render(VisionAssignmentsEditor, {
      props: { value: rows, modes, onCommit },
      context: setAppContextForTest({}),
    });
    await fireEvent.change(getByTestId("vision-mode-0"), { target: { value: "tremorsense" } });
    expect(onCommit).toHaveBeenCalledWith([{ mode: "tremorsense", range: 12 }, rows[1]]);
  });

  it("commits range edits: a number overrides, an emptied input inherits the mode default (null)", async () => {
    const onCommit = vi.fn();
    const { getByTestId } = render(VisionAssignmentsEditor, {
      props: { value: [darkvision], modes, onCommit },
      context: setAppContextForTest({}),
    });
    await fireEvent.change(getByTestId("vision-range-0"), { target: { value: "30" } });
    expect(onCommit).toHaveBeenCalledWith([{ mode: "darkvision", range: 30 }]);
    await fireEvent.change(getByTestId("vision-range-0"), { target: { value: "" } });
    expect(onCommit).toHaveBeenCalledWith([{ mode: "darkvision", range: null }]);
  });

  it("add appends a row for the first offered mode; remove drops the row, empty list included", async () => {
    const onCommit = vi.fn();
    const { getByTestId } = render(VisionAssignmentsEditor, {
      props: { value: [darkvision], modes, onCommit },
      context: setAppContextForTest({}),
    });
    await fireEvent.click(getByTestId("vision-add"));
    // SEED_VISION_MODES's first key is the "normal" mode; the appended row inherits its range.
    expect(onCommit).toHaveBeenCalledWith([darkvision, { mode: "normal", range: null }]);
    await fireEvent.click(getByTestId("vision-remove-0"));
    expect(onCommit).toHaveBeenCalledWith([]);
  });

  it("a dangling mode id (removed from the registry) stays visible as a raw option", () => {
    const { getByTestId } = render(VisionAssignmentsEditor, {
      props: { value: [{ mode: "blindsense", range: 6 }], modes, onCommit: vi.fn() },
      context: setAppContextForTest({}),
    });
    const select = getByTestId("vision-mode-0") as HTMLSelectElement;
    expect(select.value).toBe("blindsense");
    expect([...select.options].map((o) => o.value)).toContain("blindsense");
  });

  it("disabled mode renders read-only controls and a disabled add button", () => {
    const { getByTestId } = render(VisionAssignmentsEditor, {
      props: { value: [darkvision], modes, disabled: true, onCommit: vi.fn() },
      context: setAppContextForTest({}),
    });
    expect((getByTestId("vision-mode-0") as HTMLSelectElement).disabled).toBe(true);
    expect((getByTestId("vision-range-0") as HTMLInputElement).disabled).toBe(true);
    expect((getByTestId("vision-remove-0") as HTMLButtonElement).disabled).toBe(true);
    expect((getByTestId("vision-add") as HTMLButtonElement).disabled).toBe(true);
  });

  it("with no modes on offer the add button is disabled", () => {
    const { getByTestId } = render(VisionAssignmentsEditor, {
      props: { value: [], modes: [], onCommit: vi.fn() },
      context: setAppContextForTest({}),
    });
    expect((getByTestId("vision-add") as HTMLButtonElement).disabled).toBe(true);
  });
});
