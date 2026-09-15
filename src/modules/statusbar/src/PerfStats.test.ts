import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { PerformanceController } from "@shadowcat/ui-kit";
import PerfStats from "./PerfStats.svelte";

describe("PerfStats", () => {
  it("renders nothing when showStats is off", () => {
    const controller = new PerformanceController();
    render(PerfStats, { context: setAppContextForTest({ performance: controller }) });
    expect(screen.queryByTestId("perf-stats")).toBeNull();
  });

  it("shows fps/frameMs when showStats is on", () => {
    const controller = new PerformanceController();
    controller.setShowStats(true);
    controller.recordStats({ fps: 60, frameMs: 4 });
    render(PerfStats, { context: setAppContextForTest({ performance: controller }) });
    expect(screen.getByTestId("perf-stats").textContent).toContain("60");
  });
});
