import { describe, it, expect } from "vitest";
import { ContributionRegistry, sheetContract } from "@shadowcat/core";
import { sheetTable } from "./index";

describe("module-sheet-table registration", () => {
  it("registers TableSheet under shadowcat.sheet:table at priority 0", () => {
    const contributions = new ContributionRegistry();
    sheetTable.register({
      contributions: { contribute: (c: Parameters<typeof contributions.contribute>[0]) => contributions.contribute(c, { module: "sheet-table" }) },
    } as never);
    const entry = contributions.entriesFor(sheetContract("table"))[0];
    expect(entry?.contribution.sheet?.priority).toBe(0);
    expect(entry?.module).toBe("sheet-table");
  });
});
