import { describe, test, expect } from "vitest";
import { buildNoteDoc, parseNoteBody, NOTE_DOC_TYPE } from "./note-docs";
import type { WireDocument } from "./wire";

describe("buildNoteDoc", () => {
  test("builds a root-level note with the given source and defaults", () => {
    const doc = buildNoteDoc("w1", "Session 1", "# Hi");
    expect(doc.doc_type).toBe(NOTE_DOC_TYPE);
    expect(doc.name).toBe("Session 1");
    expect(doc.parent_id).toBeNull();
    expect(doc.system).toEqual({});
    expect(doc.engine).toEqual({ source: "# Hi", body: [], sort: 0n });
    expect(doc.scope).toEqual({ kind: "world", world_id: "w1" });
  });

  test("defaults to private-by-default permissions with no owner given", () => {
    const doc = buildNoteDoc("w1", null, "hello");
    expect(doc.permissions.default).toBe("none");
    expect(doc.permissions.users).toEqual({});
  });

  test("grants the author Owner when opts.owner is given", () => {
    const doc = buildNoteDoc("w1", null, "hello", { owner: "gm-1" });
    expect(doc.permissions.default).toBe("none");
    expect(doc.permissions.users).toEqual({ "gm-1": "owner" });
  });

  test("carries parentId and sort into the engine body and envelope", () => {
    const doc = buildNoteDoc("w1", null, "hello", { parentId: "parent-1", sort: 3 });
    expect(doc.parent_id).toBe("parent-1");
    expect(doc.engine).toEqual({ source: "hello", body: [], sort: 3n });
  });

  test("uses the explicit id when given", () => {
    const doc = buildNoteDoc("w1", null, "hello", { id: "n1" });
    expect(doc.id).toBe("n1");
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
