import { describe, test, expect } from "vitest";
import { buildNoteDoc, parseNoteBody, parseNotePageKind, appendNoteSortKey, NOTE_DOC_TYPE } from "./note-docs";
import { AUTHOR_CAPS } from "./scene-docs";
import type { WireDocument } from "./wire";

describe("buildNoteDoc", () => {
  test("builds a root-level note with the given source and defaults", () => {
    const doc = buildNoteDoc("w1", "Session 1", "# Hi");
    expect(doc.doc_type).toBe(NOTE_DOC_TYPE);
    expect(doc.name).toBe("Session 1");
    expect(doc.parent_id).toBeNull();
    expect(doc.system).toEqual({});
    expect(doc.engine).toEqual({ source: "# Hi", body: [], sort: 0, kind: { kind: "text" } });
    expect(doc.scope).toEqual({ kind: "world", world_id: "w1" });
  });

  test("defaults to private-by-default permissions with no owner given", () => {
    const doc = buildNoteDoc("w1", null, "hello");
    expect(doc.permissions.default).toBe("none");
    expect(doc.permissions.users).toEqual({});
  });

  test("grants the author Owner plus AUTHOR_CAPS when opts.owner is given", () => {
    const doc = buildNoteDoc("w1", null, "hello", { owner: "gm-1" });
    expect(doc.permissions.default).toBe("none");
    expect(doc.permissions.users).toEqual({ "gm-1": "owner" });
    for (const cap of AUTHOR_CAPS) {
      expect(doc.permissions.capabilities.by_role.owner).toContain(cap);
    }
  });

  test("carries parentId and sort into the engine body and envelope", () => {
    const doc = buildNoteDoc("w1", null, "hello", { parentId: "parent-1", sort: 3 });
    expect(doc.parent_id).toBe("parent-1");
    expect(doc.engine).toEqual({ source: "hello", body: [], sort: 3, kind: { kind: "text" } });
  });

  test("carries opts.kind into the engine body", () => {
    const doc = buildNoteDoc("w1", null, "caption", { kind: { kind: "pdf", asset_id: "a1" } });
    expect(doc.engine).toEqual({ source: "caption", body: [], sort: 0, kind: { kind: "pdf", asset_id: "a1" } });
  });

  test("uses the explicit id when given", () => {
    const doc = buildNoteDoc("w1", null, "hello", { id: "n1" });
    expect(doc.id).toBe("n1");
  });

  test("the whole document is JSON.stringify-friendly (no bigint reaches the wire)", () => {
    const doc = buildNoteDoc("w1", null, "hello", { sort: 5 });
    expect(() => JSON.stringify(doc)).not.toThrow();
  });
});

describe("appendNoteSortKey", () => {
  test("allocates the density constant for the first child of an empty parent", () => {
    expect(appendNoteSortKey([], null)).toBe(1000);
  });

  test("allocates one density step past the highest existing sibling, ignoring other doc_types and other parents", () => {
    const a = buildNoteDoc("w1", "A", "", { id: "a", sort: 0 });
    const b = buildNoteDoc("w1", "B", "", { id: "b", sort: 1000 });
    const child = buildNoteDoc("w1", "C", "", { id: "c", parentId: "a", sort: 500 });
    const other = { ...buildNoteDoc("w1", "Other", "", { id: "o" }), doc_type: "item" };
    expect(appendNoteSortKey([a, b, child, other], null)).toBe(2000);
    expect(appendNoteSortKey([a, b, child, other], "a")).toBe(1500);
  });
});

function noteDocWithEngine(engine: unknown): WireDocument {
  const doc = buildNoteDoc("w1", null, "hello");
  doc.engine = engine;
  return doc;
}

describe("parseNoteBody", () => {
  test("returns the segments for a valid body", () => {
    const doc = noteDocWithEngine({
      source: "hello",
      body: [{ kind: "html", sanitized_html: "<p>hello</p>" }],
      sort: 0,
    });
    expect(parseNoteBody(doc)).toEqual([{ kind: "html", sanitized_html: "<p>hello</p>" }]);
  });

  test("returns null for a wrong doc_type", () => {
    const doc = noteDocWithEngine({ source: "hello", body: [], sort: 0 });
    doc.doc_type = "item";
    expect(parseNoteBody(doc)).toBeNull();
  });

  test("returns null for a malformed known-kind segment", () => {
    const doc = noteDocWithEngine({
      source: "hello",
      body: [{ kind: "html" }], // missing sanitized_html
      sort: 0,
    });
    expect(parseNoteBody(doc)).toBeNull();
  });

  test("returns null when the engine body is missing entirely", () => {
    const doc = buildNoteDoc("w1", null, "hello");
    doc.engine = null;
    expect(parseNoteBody(doc)).toBeNull();
  });

  test("passes through a forward-compat unknown segment kind", () => {
    const doc = noteDocWithEngine({
      source: "hello",
      body: [{ kind: "future_kind", stuff: 1 }],
      sort: 0,
    });
    expect(parseNoteBody(doc)).toEqual([{ kind: "future_kind", stuff: 1 }]);
  });
});

describe("parseNotePageKind", () => {
  test("reads each media kind with its asset id", () => {
    for (const kind of ["image", "pdf", "video"] as const) {
      const doc = buildNoteDoc("w1", null, "c", { kind: { kind, asset_id: "a1" } });
      expect(parseNotePageKind(doc)).toEqual({ kind, asset_id: "a1" });
    }
  });

  test("a note stored without kind reads as text", () => {
    const doc = buildNoteDoc("w1", null, "c");
    doc.engine = { source: "c", body: [], sort: 0 };
    expect(parseNotePageKind(doc)).toEqual({ kind: "text" });
  });

  test("an unknown kind or a media kind without an asset id fails closed to text", () => {
    const doc = buildNoteDoc("w1", null, "c");
    doc.engine = { source: "c", body: [], sort: 0, kind: { kind: "audio", asset_id: "a1" } };
    expect(parseNotePageKind(doc)).toEqual({ kind: "text" });
    doc.engine = { source: "c", body: [], sort: 0, kind: { kind: "image" } };
    expect(parseNotePageKind(doc)).toEqual({ kind: "text" });
  });

  test("a non-note document reads as text", () => {
    const doc = buildNoteDoc("w1", null, "c", { kind: { kind: "image", asset_id: "a1" } });
    doc.doc_type = "item";
    expect(parseNotePageKind(doc)).toEqual({ kind: "text" });
  });
});
