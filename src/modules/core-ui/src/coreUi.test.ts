import { test, expect } from "vitest";
import { ContributionRegistry, type Contribution } from "@shadowcat/core";
import { coreUi } from "./index";

test("core-ui declares the region surfaces and contributes the layout into root", () => {
  const provided = (coreUi.manifest.provides ?? []).map((p) => p.contract);
  expect(provided).toContain("shadowcat.surface:root");
  expect(provided).toContain("shadowcat.surface:topbar");
  expect(provided).toContain("shadowcat.surface:sidebar");

  const contributions = new ContributionRegistry();
  // Minimal ModuleContext stand-in: only `contributions` is used by register.
  coreUi.register({
    contributions: { contribute: (c: Contribution) => contributions.contribute(c) },
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
  } as any);
  // The layout module contributes Layout into root; region content comes from the
  // per-element modules, so root is what core-ui itself fills.
  expect(contributions.contributionsFor("shadowcat.surface:root").length).toBe(1);
});
