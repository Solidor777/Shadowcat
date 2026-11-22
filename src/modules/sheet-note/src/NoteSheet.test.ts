import { describe, it, expect } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import { tick } from "svelte";
import {
  AssetResolver,
  DocumentStore,
  buildNoteDoc,
  buildChannelRegistryDoc,
  resolveCaps,
  canWritePath,
  type WireDocument,
} from "@shadowcat/core";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import NoteSheet from "./NoteSheet.svelte";

const SELF = "u-self";

function storeWith(...docs: WireDocument[]) {
  const s = new DocumentStore();
  s.applyCommand({
    seq: 1, world_id: "w1", author: "u", ts: 0,
    ops: docs.map((doc) => ({ op: "create" as const, doc })),
  });
  return s;
}

describe("NoteSheet title + body", () => {
  it("edits the title with the real pre-image", async () => {
    const calls: unknown[] = [];
    const doc = buildNoteDoc("w1", "Session 1", "# Hi", { id: "n1", owner: SELF });
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const { getByTestId } = render(NoteSheet, { props: { docId: "n1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.change(getByTestId("note-title"), { target: { value: "Session 1 recap" } });
    expect(calls).toEqual([[{ op: "update", doc_id: "n1", changes: [{ path: "/name", old: "Session 1", new: "Session 1 recap" }] }]]);
  });

  it("renders the server-derived body via SegmentList when it parses", () => {
    const doc = buildNoteDoc("w1", "Session 1", "# Hi", { id: "n1", owner: SELF });
    doc.engine = { ...(doc.engine as object), body: [{ kind: "text", text: "Hello world" }] };
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, canEdit: () => true });
    const { getByText } = render(NoteSheet, { props: { docId: "n1", systemPrefix: "/system", close: () => {} }, context });
    expect(getByText("Hello world")).toBeTruthy();
  });

  it("shows the unrenderable message when engine.body fails to parse", () => {
    const doc = buildNoteDoc("w1", "Session 1", "# Hi", { id: "n1", owner: SELF });
    // The segment list schema tolerates an unrecognized `kind` (forward-compat); a genuine
    // parse failure needs the wrong TOP-LEVEL shape (not an array at all).
    doc.engine = { ...(doc.engine as object), body: "not-an-array" };
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, canEdit: () => true });
    const { getByText } = render(NoteSheet, { props: { docId: "n1", systemPrefix: "/system", close: () => {} }, context });
    expect(getByText("sheetNote.unrenderable")).toBeTruthy();
  });
});

describe("NoteSheet draft-base edit flow", () => {
  it("Save dispatches ONE setField carrying the ORIGINAL source as old, even after a remote mutation", async () => {
    const calls: unknown[] = [];
    const doc = buildNoteDoc("w1", "Session 1", "# Original", { id: "n1", owner: SELF });
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const { getByTestId } = render(NoteSheet, { props: { docId: "n1", systemPrefix: "/system", close: () => {} }, context });

    await fireEvent.click(getByTestId("note-edit"));

    // Simulate a remote authoritative confirm changing the stored source WHILE the editor is open.
    documents.applyCommand({
      seq: 2, world_id: "w1", author: "other", ts: 0,
      ops: [{ op: "update", doc_id: "n1", changes: [{ path: "/engine/source", old: "# Original", new: "# Remotely changed" }] }],
    });

    await fireEvent.input(getByTestId("note-source"), { target: { value: "# My edit" } });
    await fireEvent.click(getByTestId("note-save"));

    expect(calls).toEqual([
      [{ op: "update", doc_id: "n1", changes: [{ path: "/engine/source", old: "# Original", new: "# My edit" }] }],
    ]);
  });

  it("shows the changedRemotely banner when the stored source diverges from the draft base", async () => {
    const doc = buildNoteDoc("w1", "Session 1", "# Original", { id: "n1", owner: SELF });
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, canEdit: () => true });
    const { getByTestId, getByText } = render(NoteSheet, { props: { docId: "n1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.click(getByTestId("note-edit"));
    documents.applyCommand({
      seq: 2, world_id: "w1", author: "other", ts: 0,
      ops: [{ op: "update", doc_id: "n1", changes: [{ path: "/engine/source", old: "# Original", new: "# Remotely changed" }] }],
    });
    await tick();
    expect(getByText("sheetNote.changedRemotely")).toBeTruthy();
  });

  it("Cancel discards the draft without dispatching", async () => {
    const calls: unknown[] = [];
    const doc = buildNoteDoc("w1", "Session 1", "# Original", { id: "n1", owner: SELF });
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, dispatchIntent: (ops) => calls.push(ops), canEdit: () => true });
    const { getByTestId, queryByTestId } = render(NoteSheet, { props: { docId: "n1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.click(getByTestId("note-edit"));
    await fireEvent.input(getByTestId("note-source"), { target: { value: "# Discarded" } });
    await fireEvent.click(getByTestId("note-cancel"));
    expect(calls).toEqual([]);
    expect(queryByTestId("note-source")).toBeNull();
  });

  it("Reload re-seeds the draft from the current stored value", async () => {
    const doc = buildNoteDoc("w1", "Session 1", "# Original", { id: "n1", owner: SELF });
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, canEdit: () => true });
    const { getByTestId, getByText } = render(NoteSheet, { props: { docId: "n1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.click(getByTestId("note-edit"));
    documents.applyCommand({
      seq: 2, world_id: "w1", author: "other", ts: 0,
      ops: [{ op: "update", doc_id: "n1", changes: [{ path: "/engine/source", old: "# Original", new: "# Remotely changed" }] }],
    });
    await fireEvent.click(getByText("sheetNote.reload"));
    expect((getByTestId("note-source") as HTMLTextAreaElement).value).toBe("# Remotely changed");
  });
});

describe("NoteSheet per-user sharing", () => {
  it("shares the note with one member through the permissions editor", async () => {
    const calls: unknown[] = [];
    const doc = buildNoteDoc("w1", "Session 1", "# Hi", { id: "n1", owner: SELF });
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({
      documents, members: new Map([["u-p", "Pia"]]), dispatchIntent: (ops) => calls.push(ops), canEdit: () => true,
    });
    const { getByTestId, getAllByTestId } = render(NoteSheet, { props: { docId: "n1", systemPrefix: "/system", close: () => {} }, context });
    expect(getByTestId("permissions-editor")).toBeTruthy();
    await fireEvent.click(getAllByTestId("user-multi-select-option")[0]);
    expect(calls).toEqual([[{ op: "update", doc_id: "n1", changes: [{ path: "/permissions/users/u-p", old: null, new: "observer" }] }]]);
  });

  it("hides the permissions editor from a viewer who may not write permissions", () => {
    const doc = buildNoteDoc("w1", "Session 1", "# Hi", { id: "n1", owner: SELF });
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, canEdit: () => false });
    const { queryByTestId } = render(NoteSheet, { props: { docId: "n1", systemPrefix: "/system", close: () => {} }, context });
    expect(queryByTestId("permissions-editor")).toBeNull();
  });
});

describe("NoteSheet heading table of contents", () => {
  const SOURCE = "# Intro\ntext\n## Details\n[jump](#details)";
  const HTML = '<h1>Intro</h1><p>text</p><h2>Details</h2><p><a href="#details">jump</a></p>';

  function mount() {
    const doc = buildNoteDoc("w1", "Session 1", SOURCE, { id: "n1", owner: SELF });
    doc.engine = { ...(doc.engine as object), body: [{ kind: "html", sanitized_html: HTML }] };
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, canEdit: () => true });
    return render(NoteSheet, { props: { docId: "n1", systemPrefix: "/system", close: () => {} }, context });
  }

  it("lists the source headings and assigns their ids to the rendered headings", async () => {
    const { getAllByTestId, getByTestId } = mount();
    await tick();
    expect(getAllByTestId("note-toc-link").map((a) => a.textContent)).toEqual(["Intro", "Details"]);
    const ids = [...getByTestId("note-body").querySelectorAll("h1, h2")].map((h) => h.id);
    expect(ids).toEqual(["intro", "details"]);
  });

  it("scrolls to the heading when a TOC link is clicked, without changing the location hash", async () => {
    const scrolled: string[] = [];
    const original = Element.prototype.scrollIntoView;
    Element.prototype.scrollIntoView = function (this: Element) { scrolled.push(this.id); };
    try {
      const { getAllByTestId } = mount();
      await tick();
      await fireEvent.click(getAllByTestId("note-toc-link")[1]);
      expect(scrolled).toEqual(["details"]);
      expect(window.location.hash).toBe("");
    } finally {
      Element.prototype.scrollIntoView = original;
    }
  });

  it("scrolls inside the note when a fragment link in the body is clicked", async () => {
    const scrolled: string[] = [];
    const original = Element.prototype.scrollIntoView;
    Element.prototype.scrollIntoView = function (this: Element) { scrolled.push(this.id); };
    try {
      const { getByTestId } = mount();
      await tick();
      const link = getByTestId("note-body").querySelector("a") as HTMLAnchorElement;
      await fireEvent.click(link);
      expect(scrolled).toEqual(["details"]);
      expect(window.location.hash).toBe("");
    } finally {
      Element.prototype.scrollIntoView = original;
    }
  });

  it("lists setext and raw-HTML headings by reading the rendered elements", async () => {
    const doc = buildNoteDoc("w1", "Session 1", "Title\n=====\n<h2>Raw *part*</h2>\ntext", { id: "n1", owner: SELF });
    doc.engine = { ...(doc.engine as object), body: [{ kind: "html", sanitized_html: "<h1>Title</h1><h2>Raw <em>part</em></h2>" }] };
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, canEdit: () => true });
    const { getAllByTestId, getByTestId } = render(NoteSheet, { props: { docId: "n1", systemPrefix: "/system", close: () => {} }, context });
    await tick();
    expect(getAllByTestId("note-toc-link").map((a) => a.textContent)).toEqual(["Title", "Raw part"]);
    expect([...getByTestId("note-body").querySelectorAll("h1, h2")].map((h) => h.id)).toEqual(["title", "raw-part"]);
  });

  it("renders no TOC for a note without headings", () => {
    const doc = buildNoteDoc("w1", "Session 1", "plain text", { id: "n1", owner: SELF });
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, canEdit: () => true });
    const { queryByTestId } = render(NoteSheet, { props: { docId: "n1", systemPrefix: "/system", close: () => {} }, context });
    expect(queryByTestId("note-toc")).toBeNull();
  });
});

describe("NoteSheet read-only for a non-writer", () => {
  it("hides the Edit control for a non-writer while still rendering the body", () => {
    const doc = buildNoteDoc("w1", "Session 1", "# Hi", { id: "n1", owner: SELF });
    doc.engine = { ...(doc.engine as object), body: [{ kind: "text", text: "Hello world" }] };
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, canEdit: () => false });
    const { queryByTestId, getByText } = render(NoteSheet, { props: { docId: "n1", systemPrefix: "/system", close: () => {} }, context });
    expect(queryByTestId("note-edit")).toBeNull();
    expect(getByText("Hello world")).toBeTruthy();
  });
});

describe("NoteSheet visibility", () => {
  it("shows the visibility select for the author (grantAuthor's core:edit_permissions), gated by the REAL resolveCaps + canWritePath", async () => {
    const calls: unknown[] = [];
    const doc = buildNoteDoc("w1", "Session 1", "# Hi", { id: "n1", owner: SELF });
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({
      documents,
      dispatchIntent: (ops) => calls.push(ops),
      role: "player",
      selfId: SELF,
      canEdit: (d, path) => canWritePath(path, resolveCaps(d.permissions, SELF, "player", { by_role: {}, by_user: {} }), false, []),
    });
    const { getByTestId } = render(NoteSheet, { props: { docId: "n1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.change(getByTestId("note-visibility"), { target: { value: "observer" } });
    expect(calls).toEqual([
      [{ op: "update", doc_id: "n1", changes: [{ path: "/permissions/default", old: "none", new: "observer" }] }],
    ]);
  });

  it("hides the visibility select for a non-author observer, using the same real gate", () => {
    const doc = buildNoteDoc("w1", "Session 1", "# Hi", { id: "n1", owner: SELF });
    doc.permissions.default = "observer";
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({
      documents,
      role: "player",
      selfId: "someone-else",
      canEdit: (d, path) => canWritePath(path, resolveCaps(d.permissions, "someone-else", "player", { by_role: {}, by_user: {} }), false, []),
    });
    const { queryByTestId } = render(NoteSheet, { props: { docId: "n1", systemPrefix: "/system", close: () => {} }, context });
    expect(queryByTestId("note-visibility")).toBeNull();
  });
});

describe("NoteSheet tree navigation", () => {
  it("lists children sorted by (engine.sort, created_at) and opens one on click", async () => {
    const opened: unknown[] = [];
    const parent = buildNoteDoc("w1", "Parent", "", { id: "p1", owner: SELF });
    const childB = buildNoteDoc("w1", "B", "", { id: "cb", parentId: "p1", sort: 1 });
    const childA = buildNoteDoc("w1", "A", "", { id: "ca", parentId: "p1", sort: 0 });
    const documents = storeWith(parent, childB, childA, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, canEdit: () => true, openDocument: (ref) => opened.push(ref) });
    const { getAllByTestId } = render(NoteSheet, { props: { docId: "p1", systemPrefix: "/system", close: () => {} }, context });
    const rows = getAllByTestId("note-child");
    expect(rows.map((r) => r.textContent)).toEqual(["A", "B"]);
    await fireEvent.click(rows[0]);
    expect(opened).toEqual([{ docId: "ca" }]);
  });

  it("New child note creates a child and opens it, gated by canCreate", async () => {
    const calls: unknown[] = [];
    const opened: unknown[] = [];
    const parent = buildNoteDoc("w1", "Parent", "", { id: "p1", owner: SELF });
    const documents = storeWith(parent, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({
      documents,
      canEdit: () => true,
      canCreate: () => true,
      selfId: SELF,
      world: "w1",
      dispatchIntent: (ops) => calls.push(ops),
      openDocument: (ref) => opened.push(ref),
    });
    const { getByTestId } = render(NoteSheet, { props: { docId: "p1", systemPrefix: "/system", close: () => {} }, context });
    await fireEvent.click(getByTestId("note-new-child"));
    expect(calls).toHaveLength(1);
    const created = (calls[0] as [{ op: string; doc: WireDocument }])[0];
    expect(created.op).toBe("create");
    expect(created.doc.parent_id).toBe("p1");
    expect(opened).toEqual([{ docId: created.doc.id }]);
  });

  it("hides New child note when canCreate is false", () => {
    const parent = buildNoteDoc("w1", "Parent", "", { id: "p1", owner: SELF });
    const documents = storeWith(parent, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, canEdit: () => true, canCreate: () => false });
    const { queryByTestId } = render(NoteSheet, { props: { docId: "p1", systemPrefix: "/system", close: () => {} }, context });
    expect(queryByTestId("note-new-child")).toBeNull();
  });
});

describe("NoteSheet typed pages", () => {
  const ASSET = "00000000-0000-0000-0000-0000000000a1";
  const NEW_ASSET = "00000000-0000-0000-0000-0000000000a2";

  function sheetFor(kind: { kind: "text" } | { kind: "image" | "pdf" | "video"; asset_id: string }, over: Parameters<typeof setAppContextForTest>[0] = {}) {
    const doc = buildNoteDoc("w1", "Page", "A caption", { id: "n1", owner: SELF, kind });
    const documents = storeWith(doc, buildChannelRegistryDoc("w1", { general: { name: "General" } }));
    const context = setAppContextForTest({ documents, canEdit: () => true, ...over });
    return render(NoteSheet, { props: { docId: "n1", systemPrefix: "/system", close: () => {} }, context });
  }

  it("a text page renders no media block", () => {
    const { queryByTestId } = sheetFor({ kind: "text" });
    expect(queryByTestId("note-media")).toBeNull();
  });

  it("an image page renders an img from the asset resolver above the caption body", () => {
    const { getByTestId, container } = sheetFor({ kind: "image", asset_id: ASSET });
    const media = getByTestId("note-media");
    expect(media.dataset.mediaKind).toBe("image");
    expect(media.querySelector("img")?.getAttribute("src")).toBe(`/api/assets/${ASSET}`);
    const body = getByTestId("note-body");
    expect(media.compareDocumentPosition(body) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(container.querySelector("embed, video")).toBeNull();
  });

  it("a pdf page renders an embed and a video page renders a video element", () => {
    const pdf = sheetFor({ kind: "pdf", asset_id: ASSET });
    expect(pdf.getByTestId("note-media").querySelector("embed")?.getAttribute("src")).toBe(`/api/assets/${ASSET}`);
    pdf.unmount();
    const video = sheetFor({ kind: "video", asset_id: ASSET });
    expect(video.getByTestId("note-media").querySelector("video")?.getAttribute("src")).toBe(`/api/assets/${ASSET}`);
  });

  it("an asset the resolver reports deleted renders the placeholder text, not a media element", () => {
    const assets = new AssetResolver();
    assets.onAssetChanged({ uuid: ASSET, op: "deleted", version: 2 });
    const { getByTestId, container } = sheetFor({ kind: "image", asset_id: ASSET }, { assets });
    expect(getByTestId("note-media-missing")).toBeTruthy();
    expect(container.querySelector("img")).toBeNull();
  });

  it("a media element that fails to load swaps to the placeholder text", async () => {
    const { getByTestId, container } = sheetFor({ kind: "image", asset_id: ASSET });
    await fireEvent.error(container.querySelector("img") as HTMLImageElement);
    expect(getByTestId("note-media-missing")).toBeTruthy();
  });

  it("Change media writes the whole kind with the picked asset and the old kind as the pre-image", async () => {
    const calls: unknown[] = [];
    const picks: unknown[] = [];
    const { getByTestId } = sheetFor({ kind: "pdf", asset_id: ASSET }, {
      dispatchIntent: (ops) => calls.push(ops),
      pickAsset: ((opts: unknown) => { picks.push(opts); return Promise.resolve(NEW_ASSET); }) as never,
    });
    await fireEvent.click(getByTestId("note-change-media"));
    await tick();
    expect(picks).toEqual([{ kind: "other" }]);
    expect(calls).toEqual([[{ op: "update", doc_id: "n1", changes: [{
      path: "/engine/kind",
      old: { kind: "pdf", asset_id: ASSET },
      new: { kind: "pdf", asset_id: NEW_ASSET },
    }] }]]);
  });

  it("an image page opens the picker filtered to images; a cancelled pick writes nothing", async () => {
    const calls: unknown[] = [];
    const picks: unknown[] = [];
    const { getByTestId } = sheetFor({ kind: "image", asset_id: ASSET }, {
      dispatchIntent: (ops) => calls.push(ops),
      pickAsset: ((opts: unknown) => { picks.push(opts); return Promise.resolve(null); }) as never,
    });
    await fireEvent.click(getByTestId("note-change-media"));
    await tick();
    expect(picks).toEqual([{ kind: "image" }]);
    expect(calls).toEqual([]);
  });

  it("hides Change media when the viewer cannot edit the kind", () => {
    const { queryByTestId } = sheetFor({ kind: "image", asset_id: ASSET }, { canEdit: () => false });
    expect(queryByTestId("note-change-media")).toBeNull();
  });
});
