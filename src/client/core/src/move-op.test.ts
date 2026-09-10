// @vitest-environment node
import { describe, it, expect } from "vitest";
import { buildMoveOp } from "./move-op";

describe("buildMoveOp", () => {
  it("carries the target and the true OCC pre-image", () => {
    expect(buildMoveOp("child-z", null, "root-a")).toEqual({
      op: "move",
      doc_id: "child-z",
      parent_id: null,
      old_parent_id: "root-a",
    });
  });
});
