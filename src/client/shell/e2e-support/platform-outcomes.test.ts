// @vitest-environment node
import type { TestInfo } from "@playwright/test";
import { describe, expect, it } from "vitest";
import { checkMeasuredOutcome, type RunEnvironment } from "./platform-outcomes";

const TABLE = { "project-a": { win32: "blocked" } } as const;

/** A `TestInfo` carrying only what `checkMeasuredOutcome` reads and writes. */
function stubInfo(project = "project-a"): TestInfo & { annotations: { type: string; description?: string }[] } {
  return { annotations: [], project: { name: project } } as unknown as TestInfo & {
    annotations: { type: string; description?: string }[];
  };
}

const local = (platform: RunEnvironment["platform"]): RunEnvironment => ({ platform, ci: false });
const ci = (platform: RunEnvironment["platform"]): RunEnvironment => ({ platform, ci: true });

describe("checkMeasuredOutcome", () => {
  it("returns true and records the observation when it equals the measured outcome", () => {
    const info = stubInfo();
    expect(checkMeasuredOutcome(info, TABLE, "blocked", local("win32"))).toBe(true);
    expect(info.annotations).toEqual([{ type: "observed-outcome", description: "win32: blocked" }]);
  });

  it("throws when the observation differs from the measured outcome", () => {
    expect(() => checkMeasuredOutcome(stubInfo(), TABLE, "frame", local("win32"))).toThrow(/"frame".*"blocked"/);
  });

  it("annotates an unmeasured cell locally and returns false without throwing", () => {
    const info = stubInfo();
    expect(checkMeasuredOutcome(info, TABLE, "frame", local("linux"))).toBe(false);
    expect(info.annotations.map((a) => a.type)).toEqual(["observed-outcome", "UNMEASURED"]);
  });

  it("throws on an unmeasured cell on CI, carrying the observed value", () => {
    expect(() => checkMeasuredOutcome(stubInfo(), TABLE, "certificate-error", ci("linux"))).toThrow(
      /"certificate-error"/,
    );
  });
});
