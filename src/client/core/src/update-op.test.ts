// @vitest-environment node
import { describe, it, expect } from "vitest";
import { buildUpdate } from "./update-op";

describe("buildUpdate", () => {
  it("builds a single-edit Update", () => {
    expect(buildUpdate("doc-1", [{ path: "/system/hp", old: 10, value: 12 }])).toEqual({
      op: "update",
      doc_id: "doc-1",
      changes: [{ path: "/system/hp", old: 10, new: 12 }],
    });
  });

  it("preserves edit order across multiple changes", () => {
    expect(
      buildUpdate("doc-1", [
        { path: "/engine/x", old: 0, value: 5 },
        { path: "/engine/y", old: 0, value: 7 },
      ]),
    ).toEqual({
      op: "update",
      doc_id: "doc-1",
      changes: [
        { path: "/engine/x", old: 0, new: 5 },
        { path: "/engine/y", old: 0, new: 7 },
      ],
    });
  });

  it("collapses a genuinely absent (undefined) pre-image to null", () => {
    expect(buildUpdate("doc-1", [{ path: "/system/flag", old: undefined, value: true }])).toEqual({
      op: "update",
      doc_id: "doc-1",
      changes: [{ path: "/system/flag", old: null, new: true }],
    });
  });

  it("preserves a falsy real old value verbatim", () => {
    expect(buildUpdate("doc-1", [{ path: "/system/count", old: 0, value: 1 }])).toEqual({
      op: "update",
      doc_id: "doc-1",
      changes: [{ path: "/system/count", old: 0, new: 1 }],
    });
    expect(buildUpdate("doc-1", [{ path: "/system/on", old: false, value: true }])).toEqual({
      op: "update",
      doc_id: "doc-1",
      changes: [{ path: "/system/on", old: false, new: true }],
    });
    expect(buildUpdate("doc-1", [{ path: "/system/label", old: "", value: "x" }])).toEqual({
      op: "update",
      doc_id: "doc-1",
      changes: [{ path: "/system/label", old: "", new: "x" }],
    });
  });

  it("builds a remove-shaped change", () => {
    expect(buildUpdate("doc-1", [{ path: "/system/tempFlag", old: true, remove: true }])).toEqual({
      op: "update",
      doc_id: "doc-1",
      changes: [{ path: "/system/tempFlag", old: true, new: null, remove: true }],
    });
  });
});
