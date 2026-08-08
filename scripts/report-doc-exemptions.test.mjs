import { describe, expect, it } from "vitest";
import { reportDocExemptions } from "./report-doc-exemptions.mjs";

describe("reportDocExemptions", () => {
  it("counts the enumerated exemptions", () => {
    const result = reportDocExemptions({ intentionallyNotDocumented: ["A.x", "B.y"] });
    expect(result.count).toBe(2);
    expect(result.names).toEqual(["A.x", "B.y"]);
  });

  it("reports zero when the key is absent", () => {
    expect(reportDocExemptions({}).count).toBe(0);
  });
});
