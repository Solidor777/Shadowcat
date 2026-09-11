import { describe, it, expect } from "vitest";
import { ContributionRegistry, sheetContract } from "@shadowcat/core";
import { sheetNote } from "./index";

describe("module-sheet-note registration", () => {
  it("registers NoteSheet under shadowcat.sheet:note at priority 0", () => {
    const contributions = new ContributionRegistry();
    sheetNote.register({
      contributions: { contribute: (c: Parameters<typeof contributions.contribute>[0]) => contributions.contribute(c, { module: "sheet-note" }) },
    } as never);
    const entry = contributions.entriesFor(sheetContract("note"))[0];
    expect(entry?.contribution.sheet?.priority).toBe(0);
    expect(entry?.module).toBe("sheet-note");
  });
});
