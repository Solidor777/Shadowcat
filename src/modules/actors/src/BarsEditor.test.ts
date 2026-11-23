import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { MAX_BARS, type BarAttribute } from "@shadowcat/core";
import BarsEditor from "./BarsEditor.svelte";

const hp = (): BarAttribute => ({ label: "HP", current: "hp.value", max: 10, color: "#cc2222", mode: "always" });

function mount(bars: BarAttribute[], onCommit = vi.fn()) {
  render(BarsEditor, { context: setAppContextForTest({}), props: { bars, onCommit } });
  return onCommit;
}

describe("BarsEditor", () => {
  it("shows each bar's authored fields", () => {
    mount([hp()]);
    expect((screen.getByTestId("bar-label") as HTMLInputElement).value).toBe("HP");
    expect((screen.getByTestId("bar-current") as HTMLInputElement).value).toBe("hp.value");
    expect((screen.getByTestId("bar-max") as HTMLInputElement).value).toBe("10");
    expect((screen.getByTestId("bar-color") as HTMLInputElement).value).toBe("#cc2222");
    expect((screen.getByTestId("bar-mode") as HTMLSelectElement).value).toBe("always");
  });

  it("a numeric current commits as a number, a formula as trimmed text, whole list per edit", async () => {
    const onCommit = mount([hp()]);
    await fireEvent.change(screen.getByTestId("bar-current"), { target: { value: " 7 " } });
    expect(onCommit).toHaveBeenLastCalledWith([{ ...hp(), current: 7 }]);
    await fireEvent.change(screen.getByTestId("bar-max"), { target: { value: "hp.max" } });
    expect(onCommit).toHaveBeenLastCalledWith([{ ...hp(), max: "hp.max" }]);
  });

  it("an invalid formula shows an inline error and commits nothing", async () => {
    const onCommit = mount([hp()]);
    await fireEvent.change(screen.getByTestId("bar-current"), { target: { value: "1 +" } });
    expect(onCommit).not.toHaveBeenCalled();
    expect(screen.getByRole("alert").textContent).toContain("actors.barInvalidFormula");
  });

  it("an empty label is rejected inline", async () => {
    const onCommit = mount([hp()]);
    await fireEvent.change(screen.getByTestId("bar-label"), { target: { value: "  " } });
    expect(onCommit).not.toHaveBeenCalled();
    expect(screen.getByRole("alert").textContent).toContain("actors.barLabelRequired");
  });

  it("colour and mode edits commit the list", async () => {
    const onCommit = mount([hp()]);
    await fireEvent.change(screen.getByTestId("bar-color"), { target: { value: "#00ff00" } });
    expect(onCommit).toHaveBeenLastCalledWith([{ ...hp(), color: "#00ff00" }]);
    await fireEvent.change(screen.getByTestId("bar-mode"), { target: { value: "owner" } });
    expect(onCommit).toHaveBeenLastCalledWith([{ ...hp(), mode: "owner" }]);
  });

  it("add appends a default bar; remove drops one", async () => {
    const onCommit = mount([hp()]);
    await fireEvent.click(screen.getByTestId("bar-add"));
    expect(onCommit.mock.calls[0][0]).toHaveLength(2);
    expect(onCommit.mock.calls[0][0][1]).toMatchObject({ label: "actors.barDefaultLabel", mode: "hover" });
    await fireEvent.click(screen.getByTestId("bar-remove"));
    expect(onCommit).toHaveBeenLastCalledWith([]);
  });

  it("add is disabled at the bar cap", () => {
    mount(Array.from({ length: MAX_BARS }, hp));
    expect((screen.getByTestId("bar-add") as HTMLButtonElement).disabled).toBe(true);
  });
});
