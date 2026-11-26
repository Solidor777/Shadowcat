import { describe, it, expect, vi } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import LightEmissionEditor from "./LightEmissionEditor.svelte";
import { setAppContextForTest } from "./__fixtures__/appContextTest";
import type { LightEmission } from "@shadowcat/core";

const torch: LightEmission = { color: "#ffcc66", intensity: 1, brightRadius: 2, dimRadius: 4, falloff: null, enabled: true, rotation: 0, followTokenRotation: false };

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

  it("the darkness window checkbox writes a threshold and unchecking removes the key", async () => {
    const onCommit = vi.fn();
    const first = render(LightEmissionEditor, {
      props: { value: torch, onCommit },
      context: setAppContextForTest({}),
    });
    await fireEvent.click(first.getByTestId("emission-darkness-only"));
    expect(onCommit).toHaveBeenLastCalledWith({ ...torch, activeBelowEnv: 0.5 });
    first.unmount();
    onCommit.mockClear();
    const { getByTestId: on } = render(LightEmissionEditor, {
      props: { value: { ...torch, activeBelowEnv: 0.3 }, onCommit },
      context: setAppContextForTest({}),
    });
    await fireEvent.click(on("emission-darkness-only"));
    const written = onCommit.mock.calls[0][0] as LightEmission;
    expect("activeBelowEnv" in written).toBe(false);
  });

  it("the darkness threshold slider clamps into [0, 1]", async () => {
    const onCommit = vi.fn();
    const { getByTestId } = render(LightEmissionEditor, {
      props: { value: { ...torch, activeBelowEnv: 0.3 }, onCommit },
      context: setAppContextForTest({}),
    });
    const slider = getByTestId("emission-darkness-threshold") as HTMLInputElement;
    // A range input sanitises its own value, so drive the handler with a value the DOM keeps.
    Object.defineProperty(slider, "value", { configurable: true, value: "7" });
    await fireEvent.change(slider);
    expect(onCommit).toHaveBeenLastCalledWith({ ...torch, activeBelowEnv: 1 });
    Object.defineProperty(slider, "value", { configurable: true, value: "-2" });
    await fireEvent.change(slider);
    expect(onCommit).toHaveBeenLastCalledWith({ ...torch, activeBelowEnv: 0 });
  });

  it("the cone checkbox reveals angle and rotation, and unchecking removes the angle", async () => {
    const onCommit = vi.fn();
    const plain = render(LightEmissionEditor, {
      props: { value: torch, onCommit },
      context: setAppContextForTest({}),
    });
    expect(plain.queryByTestId("emission-cone-angle")).toBeNull();
    await fireEvent.click(plain.getByTestId("emission-cone"));
    expect(onCommit).toHaveBeenLastCalledWith({ ...torch, angle: 90 });
    plain.unmount();
    onCommit.mockClear();
    const cone = { ...torch, angle: 90 };
    const r = render(LightEmissionEditor, { props: { value: cone, onCommit }, context: setAppContextForTest({}) });
    await fireEvent.change(r.getByTestId("emission-cone-angle"), { target: { value: "400" } });
    expect(onCommit).toHaveBeenLastCalledWith({ ...cone, angle: 360 });
    await fireEvent.change(r.getByTestId("emission-cone-angle"), { target: { value: "0" } });
    expect(onCommit).toHaveBeenCalledTimes(1);
    await fireEvent.change(r.getByTestId("emission-cone-rotation"), { target: { value: "-30" } });
    expect(onCommit).toHaveBeenLastCalledWith({ ...cone, rotation: -30 });
    await fireEvent.click(r.getByTestId("emission-cone"));
    const written = onCommit.mock.calls[onCommit.mock.calls.length - 1][0] as LightEmission;
    expect("angle" in written).toBe(false);
  });

  it("the follow-token-facing control appears only for a carried cone light", async () => {
    const onCommit = vi.fn();
    const cone = { ...torch, angle: 60 };
    const standalone = render(LightEmissionEditor, { props: { value: cone, onCommit }, context: setAppContextForTest({}) });
    expect(standalone.queryByTestId("emission-follow-token")).toBeNull();
    standalone.unmount();
    const carried = render(LightEmissionEditor, {
      props: { value: cone, carriedByToken: true, onCommit },
      context: setAppContextForTest({}),
    });
    await fireEvent.click(carried.getByTestId("emission-follow-token"));
    expect(onCommit).toHaveBeenLastCalledWith({ ...cone, followTokenRotation: true });
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
