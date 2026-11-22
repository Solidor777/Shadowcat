import type { PresentNoteNotice, ReadableDocuments, NotificationLevel, SheetRef } from "@shadowcat/core";

/** Collaborators `createPresentedNoteHandler` needs. */
export interface PresentedNoteDeps {
  /** The reactive document mirror the presented note is resolved against. */
  documents: ReadableDocuments;
  /** This client's user id; the presenting GM's own echo is ignored. */
  selfId: string;
  /** Opens (or focuses) a document's sheet panel.
   * @param ref The document to open. */
  openDocument: (ref: SheetRef) => void;
  /** Raises a transient notification.
   * @param message The notification text.
   * @param level The severity; the host default applies when omitted. */
  notify: (message: string, level?: NotificationLevel) => void;
  /** Translates an i18n key.
   * @param key The i18n key.
   * @param params Values substituted into the message.
   * @returns The translated text. */
  t: (key: string, params?: Record<string, string | number>) => string;
}

/**
 * Builds the handler for a relayed `PresentNoteNotice`: opens the presented note's sheet panel
 * and raises a transient "presenting" notification. The presenting GM's own echo is ignored. A
 * note this client cannot resolve (no read access, or deleted before the signal arrived) opens
 * nothing and raises nothing, so the signal fails closed without revealing the note.
 * @param deps The document mirror, identity and UI seams the handler acts through.
 * @returns A handler suitable for `WorldSession.onPresentNote`.
 * @example
 * ```ts
 * import { createPresentedNoteHandler } from "./presentedNote";
 *
 * declare const deps: Parameters<typeof createPresentedNoteHandler>[0];
 * const handle = createPresentedNoteHandler(deps);
 * handle({ noteId: "note-1", presentedBy: "gm-1" });
 * ```
 */
export function createPresentedNoteHandler(deps: PresentedNoteDeps): (msg: PresentNoteNotice) => void {
  return (msg) => {
    if (msg.presentedBy === deps.selfId) return;
    const doc = deps.documents.get(msg.noteId);
    if (!doc) return;
    deps.openDocument({ docId: msg.noteId });
    deps.notify(deps.t("notes.presenting", { name: doc.name ?? deps.t("sheetNote.untitled") }), "info");
  };
}
