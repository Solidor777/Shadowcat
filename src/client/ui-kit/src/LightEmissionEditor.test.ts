import { describe, it, expect, vi } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import LightEmissionEditor from "./LightEmissionEditor.svelte";
import { setAppContextForTest } from "./__fixtures__/appContextTest";
import type { LightEmission } from "@shadowcat/core";

const torch: LightEmission = { color: "#ffcc66", intensity: 1, brightRadius: 2, dimRadius: 4, falloff: null, enabled: true };

describe("LightEmissionEditor", () => {
  it("commits whole-payload updates: one field changes, the rest are preserved", async () => {
    const onCommit = vi.fn();
    const { getByTestId } = render(LightEmissionEditor, {
      props: { value: torch, onCommit },
      context: setAppContextForTest({}),
    });
    await fireEvent.change(getByTestId("emission-intensity"), { target: { value: "0.5" } });
    expect(onCommit).toHaveBeenCalledWith({ ...torch, intensity: 0.5 });
    await fireEvent.click(getByTestId("emission-enabled"));
    expect(onCommit).toHaveBeenCalledWith({ ...torch, enabled: false });
    // An absent falloff reads as the linear default; choosing a curve writes the wrapper object.
    await fireEvent.change(getByTestId("emission-falloff"), { target: { value: "quadratic" } });
    expect(onCommit).toHaveBeenCalledWith({ ...torch, falloff: { curve: "quadratic" } });
  });

  it("ignores a non-numeric radius edit (no commit)", async () => {
    const onCommit = vi.fn();
    const { getByTestId } = render(LightEmissionEditor, {
      props: { value: torch, onCommit },
      context: setAppContextForTest({}),
    });
    await fireEvent.change(getByTestId("emission-dim"), { target: { value: "" } });
    expect(onCommit).not.toHaveBeenCalled();
  });

  it("disabled mode renders read-only controls", () => {
    const { getByTestId } = render(LightEmissionEditor, {
      props: { value: torch, disabled: true, onCommit: vi.fn() },
      context: setAppContextForTest({}),
    });
    expect((getByTestId("emission-enabled") as HTMLInputElement).disabled).toBe(true);
    expect((getByTestId("emission-falloff") as HTMLSelectElement).disabled).toBe(true);
  });
});
