import { describe, it, expect } from "vitest";
import type { SheetRef } from "@shadowcat/core";
import type { AppContext } from "@shadowcat/ui-kit";

// The AppContext type must carry `openDocument`; a compile-time surface check that the
// seam exists with the right shape (runtime wiring is exercised by the ui-kit
// SheetsController tests + the panels e2e).
describe("AppContext.openDocument seam", () => {
  it("accepts docId and tokenId refs", () => {
    const refs: SheetRef[] = [{ docId: "d1" }, { docId: "d1", embeddedPath: "/embedded/item/0" }, { tokenId: "t1" }];
    const fn: AppContext["openDocument"] = () => {};
    for (const r of refs) fn(r);
    expect(refs).toHaveLength(3);
  });
});
