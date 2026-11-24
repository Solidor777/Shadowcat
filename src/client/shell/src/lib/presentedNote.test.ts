import { describe, it, expect } from "vitest";
import { DocumentStore, buildNoteDoc } from "@shadowcat/core";
import { createPresentedNoteHandler } from "./presentedNote";

/** A handler over a store holding one note, recording every open and notification. */
function setup(name = "Handout") {
  const documents = new DocumentStore();
  documents.applyCommand({
    seq: 1, world_id: "w1", author: "u", ts: 0,
    ops: [{ op: "create", doc: buildNoteDoc("w1", name, "# Hi", { id: "n1", owner: "gm-1" }) }],
  });
  const opened: unknown[] = [];
  const notices: { message: string; level?: string }[] = [];
  const handle = createPresentedNoteHandler({
    documents,
    selfId: "player-1",
    openDocument: (ref) => opened.push(ref),
    notify: (message, level) => notices.push({ message, level }),
    t: (key, params) => (params ? `${key}:${params.name}` : key),
  });
  return { handle, opened, notices };
}

describe("createPresentedNoteHandler", () => {
  it("opens the note's sheet and raises an info notice naming it", () => {
    const { handle, opened, notices } = setup();
    handle({ noteId: "n1", presentedBy: "gm-1" });
    expect(opened).toEqual([{ docId: "n1" }]);
    expect(notices).toEqual([{ message: "notes.presenting:Handout", level: "info" }]);
  });

  it("names an empty-titled note by the untitled label", () => {
    const { handle, notices } = setup("");
    handle({ noteId: "n1", presentedBy: "gm-1" });
    expect(notices).toEqual([{ message: "notes.presenting:sheetNote.untitled", level: "info" }]);
  });

  it("ignores the presenting user's own echo", () => {
    const { handle, opened, notices } = setup();
    handle({ noteId: "n1", presentedBy: "player-1" });
    expect(opened).toEqual([]);
    expect(notices).toEqual([]);
  });

  it("opens and announces nothing for a note this client cannot resolve", () => {
    const { handle, opened, notices } = setup();
    handle({ noteId: "unreadable", presentedBy: "gm-1" });
    expect(opened).toEqual([]);
    expect(notices).toEqual([]);
  });
});
