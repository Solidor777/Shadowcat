import { describe, it, expect } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { PerformanceController } from "@shadowcat/ui-kit";
import PerformanceEditor from "./PerformanceEditor.svelte";

describe("PerformanceEditor", () => {
  function renderEditor(controller = new PerformanceController()) {
    render(PerformanceEditor, { context: setAppContextForTest({ performance: controller }) });
    return controller;
  }

  it("selecting a preset radio calls setPreset", async () => {
    const controller = renderEditor();
    await fireEvent.click(screen.getByTestId("perf-preset-mobile"));
    expect(controller.preset).toBe("mobile");
    expect(controller.current).toMatchObject({ fpsCap: 30 });
  });

  it("editing a single field shows custom", async () => {
    const controller = renderEditor();
    await fireEvent.change(screen.getByTestId("perf-fps-cap"), { target: { value: "30" } });
    expect(controller.preset).toBe("custom");
    expect(screen.getByText("performance.preset.custom")).toBeTruthy();
  });

  it("reset returns to auto", async () => {
    const controller = renderEditor();
    await fireEvent.change(screen.getByTestId("perf-fps-cap"), { target: { value: "30" } });
    await fireEvent.click(screen.getByTestId("perf-reset"));
    expect(controller.preset).toBe("auto");
  });

  it("every control is labelled", () => {
    renderEditor();
    for (const id of ["perf-fps-cap", "perf-render-scale", "perf-lighting", "perf-antialias", "perf-token-fx", "perf-vfx", "perf-dice3d", "perf-spatial-audio", "perf-idle-skip", "perf-reduced-motion", "perf-show-stats"]) {
      expect(screen.getByTestId(id).closest("label")).not.toBeNull();
    }
  });
});
