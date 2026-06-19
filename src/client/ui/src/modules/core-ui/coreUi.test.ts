import { test, expect } from "vitest";
import { ContributionRegistry } from "@shadowcat/core";
import { coreUi } from "./index";

test("core-ui declares the region surfaces and contributes default panels", () => {
  const provided = (coreUi.manifest.provides ?? []).map((p) => p.contract);
  expect(provided).toContain("shadowcat.surface:root");
  expect(provided).toContain("shadowcat.surface:sidebar");

  const contributions = new ContributionRegistry();
  // Minimal ModuleContext stand-in: only `contributions` is used by register.
  coreUi.register({
    contributions: { contribute: (c) => contributions.contribute(c) },
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
  } as any);
  expect(contributions.contributionsFor("shadowcat.surface:sidebar").length).toBeGreaterThan(0);
});
