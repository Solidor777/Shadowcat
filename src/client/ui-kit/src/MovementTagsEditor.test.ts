import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "./__fixtures__/appContextTest";
import MovementTagsEditor from "./MovementTagsEditor.svelte";

/** Render the editor with a commit spy. */
function setup(value: string[], opts: { disabled?: boolean } = {}) {
  const onCommit = vi.fn();
  render(MovementTagsEditor, {
    context: setAppContextForTest({}),
    props: { value, onCommit, ...opts },
  });
  return onCommit;
}

describe("MovementTagsEditor", () => {
  it("reflects the value on the reserved toggle chips and commits toggles as whole lists", async () => {
    const onCommit = setup(["flying"]);
    expect(screen.getByTestId("movement-toggle-flying").getAttribute("aria-pressed")).toBe("true");
    expect(screen.getByTestId("movement-toggle-incorporeal").getAttribute("aria-pressed")).toBe("false");

    await fireEvent.click(screen.getByTestId("movement-toggle-incorporeal"));
    expect(onCommit).toHaveBeenCalledWith(["flying", "incorporeal"]);

    await fireEvent.click(screen.getByTestId("movement-toggle-flying"));
    expect(onCommit).toHaveBeenCalledWith([]);
  });

  it("renders free-form tags as removable chips; removal commits the filtered list", async () => {
    const onCommit = setup(["flying", "burrowing"]);
    await fireEvent.click(screen.getByTestId("movement-remove-burrowing"));
    expect(onCommit).toHaveBeenCalledWith(["flying"]);
  });

  it("deduplicates a doubled stored tag for display and removes every occurrence at once", async () => {
    const onCommit = setup(["burrowing", "burrowing"]);
    expect(screen.getAllByTestId("movement-remove-burrowing")).toHaveLength(1);
    await fireEvent.click(screen.getByTestId("movement-remove-burrowing"));
    expect(onCommit).toHaveBeenCalledWith([]);
  });

  it("adds a trimmed free-form tag and clears the draft", async () => {
    const onCommit = setup([]);
    const input = screen.getByTestId("movement-input") as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "  burrowing  " } });
    await fireEvent.click(screen.getByTestId("movement-add"));
    expect(onCommit).toHaveBeenCalledWith(["burrowing"]);
    expect(input.value).toBe("");
  });

  it("adds via the Enter key without submitting a hosting form", async () => {
    const onCommit = setup(["flying"]);
    const input = screen.getByTestId("movement-input");
    await fireEvent.input(input, { target: { value: "swimming" } });
    await fireEvent.keyDown(input, { key: "Enter" });
    expect(onCommit).toHaveBeenCalledWith(["flying", "swimming"]);
  });

  it("refuses empty and duplicate drafts (fail-closed, no commit)", async () => {
    const onCommit = setup(["flying"]);
    const input = screen.getByTestId("movement-input");
    // Empty: the add button stays disabled.
    expect((screen.getByTestId("movement-add") as HTMLButtonElement).disabled).toBe(true);
    // Exact duplicate (reserved or custom) is a no-op.
    await fireEvent.input(input, { target: { value: " flying " } });
    await fireEvent.click(screen.getByTestId("movement-add"));
    expect(onCommit).not.toHaveBeenCalled();
  });

  it("a reserved tag typed into the add row joins the list like any other tag", async () => {
    const onCommit = setup([]);
    await fireEvent.input(screen.getByTestId("movement-input"), { target: { value: "incorporeal" } });
    await fireEvent.click(screen.getByTestId("movement-add"));
    expect(onCommit).toHaveBeenCalledWith(["incorporeal"]);
  });

  it("disables every control in read-only mode", () => {
    setup(["flying", "burrowing"], { disabled: true });
    expect((screen.getByTestId("movement-toggle-flying") as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByTestId("movement-remove-burrowing") as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByTestId("movement-input") as HTMLInputElement).disabled).toBe(true);
    expect((screen.getByTestId("movement-add") as HTMLButtonElement).disabled).toBe(true);
  });
});
