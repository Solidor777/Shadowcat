import { describe, it, expect } from "vitest";
import { ContributionRegistry, SHEET_FALLBACK_CONTRACT } from "@shadowcat/core";
import { sheetFallback } from "./index";

describe("module-sheet-fallback", () => {
  it("registers the generic fallback sheet at -Infinity priority under the fallback contract", () => {
    const contributions = new ContributionRegistry();
    // Mirrors the real ModuleContext.contributions wrapper (`ModuleRegistry.activate`):
    // a 1-arg `contribute(c)` closure that auto-injects the module id — the module entry
    // never self-declares it, matching every other first-party module.
    sheetFallback.register({
      contributions: { contribute: (c: Parameters<typeof contributions.contribute>[0]) => contributions.contribute(c, { module: "sheet-fallback" }) },
    } as never);
    const entry = contributions.entriesFor(SHEET_FALLBACK_CONTRACT)[0];
    expect(entry?.contribution.sheet?.priority).toBe(-Infinity);
    expect(entry?.module).toBe("sheet-fallback");
  });
});
