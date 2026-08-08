import { test, expect } from "vitest";
import { ContributionRegistry, type Contribution, type ModuleContext } from "@shadowcat/core";
import { coreUi } from "./index";

test("core-ui declares the region surfaces and contributes the layout into root", () => {
  const provided = (coreUi.manifest.provides ?? []).map((p) => p.contract);
  expect(provided).toContain("shadowcat.surface:root");
  expect(provided).toContain("shadowcat.surface:topbar");
  expect(provided).toContain("shadowcat.surface:panel-host");
  expect(provided).not.toContain("shadowcat.surface:sidebar-host");
  expect(provided).not.toContain("shadowcat.surface:sidebar");

  const contributions = new ContributionRegistry();
  // Minimal ModuleContext stand-in: `register` reads only `contributions`. The cast
  // names what is being stood in for, so a ModuleContext change that `register` starts
  // depending on surfaces here rather than passing through untyped.
  coreUi.register({
    contributions: { contribute: (c: Contribution) => contributions.contribute(c) },
  } as Pick<ModuleContext, "contributions"> as ModuleContext);
  // The layout module contributes Layout into root; region content comes from the
  // per-element modules, so root is what core-ui itself fills.
  expect(contributions.contributionsFor("shadowcat.surface:root").length).toBe(1);
});
