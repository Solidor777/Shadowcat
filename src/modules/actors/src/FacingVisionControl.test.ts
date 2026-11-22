import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import FacingVisionControl from "./FacingVisionControl.svelte";

function mount(value: { arc: number; near: number } | null, extra: Record<string, unknown> = {}) {
  const onCommit = vi.fn();
  render(FacingVisionControl, { context: setAppContextForTest({}), props: { value, onCommit, ...extra } });
  return onCommit;
}

describe("FacingVisionControl", () => {
  it("offers only the checkbox while the value is null, and enabling commits the wide default", async () => {
    const onCommit = mount(null);
    expect(screen.queryByTestId("facing-arc")).toBeNull();
    await fireEvent.click(screen.getByTestId("facing-enabled"));
    expect(onCommit).toHaveBeenCalledWith({ arc: 120, near: 1 });
  });

  it("shows the stored arc and awareness, and clearing the checkbox commits null", async () => {
    const onCommit = mount({ arc: 90, near: 2 });
    expect((screen.getByTestId("facing-arc") as HTMLInputElement).value).toBe("90");
    expect((screen.getByTestId("facing-arc-slider") as HTMLInputElement).value).toBe("90");
    expect((screen.getByTestId("facing-near") as HTMLInputElement).value).toBe("2");
    await fireEvent.click(screen.getByTestId("facing-enabled"));
    expect(onCommit).toHaveBeenCalledWith(null);
  });

  it("the number field and the slider each commit the whole value with the new arc", async () => {
    const onCommit = mount({ arc: 90, near: 2 });
    await fireEvent.change(screen.getByTestId("facing-arc"), { target: { value: "45" } });
    expect(onCommit).toHaveBeenLastCalledWith({ arc: 45, near: 2 });
    await fireEvent.change(screen.getByTestId("facing-arc-slider"), { target: { value: "200" } });
    expect(onCommit).toHaveBeenLastCalledWith({ arc: 200, near: 2 });
  });

  it("clamps an arc above 360 to the full circle and ignores a zero or empty arc", async () => {
    const onCommit = mount({ arc: 90, near: 2 });
    await fireEvent.change(screen.getByTestId("facing-arc"), { target: { value: "500" } });
    expect(onCommit).toHaveBeenLastCalledWith({ arc: 360, near: 2 });
    onCommit.mockClear();
    await fireEvent.change(screen.getByTestId("facing-arc"), { target: { value: "0" } });
    await fireEvent.change(screen.getByTestId("facing-arc"), { target: { value: "" } });
    expect(onCommit).not.toHaveBeenCalled();
  });

  it("commits the awareness radius with the stored arc, and ignores a negative radius", async () => {
    const onCommit = mount({ arc: 90, near: 2 });
    await fireEvent.change(screen.getByTestId("facing-near"), { target: { value: "0" } });
    expect(onCommit).toHaveBeenLastCalledWith({ arc: 90, near: 0 });
    onCommit.mockClear();
    await fireEvent.change(screen.getByTestId("facing-near"), { target: { value: "-1" } });
    expect(onCommit).not.toHaveBeenCalled();
  });

  it("hides the checkbox when not toggleable and disables every field when disabled", () => {
    mount({ arc: 90, near: 2 }, { toggleable: false, disabled: true });
    expect(screen.queryByTestId("facing-enabled")).toBeNull();
    expect((screen.getByTestId("facing-arc") as HTMLInputElement).disabled).toBe(true);
    expect((screen.getByTestId("facing-near") as HTMLInputElement).disabled).toBe(true);
  });
});
