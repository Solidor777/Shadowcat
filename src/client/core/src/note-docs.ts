// Client seam for the `note` engine document (`data::engine::note::NoteEngine`).
// Unlike `TableEngine`, a client never authors `body` meaningfully — the server
// unconditionally overwrites it on every Create/Update post-image
// (`normalize_engine`'s `"note"` arm) — so `buildNoteDoc` builds only `source`
// and sends an empty placeholder `body`, and `parseNoteBody` is the fail-closed
// reader of whatever the server actually derived and echoed back.
import { envelope, grantAuthor } from "./scene-docs";
import { SegmentListSchema, type ChatSegment, type UnknownSegment } from "./chat-docs";
import type { NoteEngine } from "@shadowcat/types";
import type { WireDocument } from "./wire";

export type { NoteEngine } from "@shadowcat/types";

/** The `doc_type` identifying a stored note document (server:
 * `data::engine::note::NOTE_DOC_TYPE`). */
export const NOTE_DOC_TYPE = "note";

/** Optional construction parameters for `buildNoteDoc`. */
export interface BuildNoteDocOptions {
  /** The parent note's id, for a child note in the note tree (`data::sqlite::notes::check_note_parent`
   * requires the parent to be another note in the same world). `null`/omitted makes a root-level note. */
  parentId?: string;
  /** Sibling ordering under `parentId` (ties broken by `created_at` server-side). Defaults to `0`. */
  sort?: number;
  /** Optional explicit document id; a fresh uuid is generated when omitted. */
  id?: string;
  /** The authoring user's id. REQUIRED to grant that user `Owner` PLUS `AUTHOR_CAPS`
   * (`core:delete`, `core:edit_permissions`) on the note via `grantAuthor` — the `owner`
   * DocRole floor alone (read + write_fields) cannot delete the note or reshare it
   * (private-by-default: `permissions.default: "none"`). This builder is pure and has no
   * session of its own to read the caller's identity from, so the caller (which does have a
   * session) must pass it explicitly. Omitting it produces a note nobody but a GM can read,
   * delete, or reshare. */
  owner?: string;
}

/** Builds an unsaved `note` document: `engine: { source, body: [], sort }` (the
 * placeholder `body` is discarded and re-derived by the server on Create — see this
 * module's own doc comment), `system: {}`, private-by-default permissions
 * (`default: "none"`, `grantAuthor`'d to `opts.owner` when given — see `BuildNoteDocOptions.owner`).
 * @param worldId The owning world's id.
 * @param name The note's display title (envelope `name`), or `null`.
 * @param source The author's markdown.
 * @param opts Optional parent, sort key, explicit id, and authoring owner.
 * @returns The unsaved `WireDocument`, ready to `Create`.
 * @example
 * ```ts
 * import { buildNoteDoc } from "@shadowcat/core";
 *
 * buildNoteDoc("00000000-0000-0000-0000-000000000001", "Session 1", "# Hi", {
 *   owner: "00000000-0000-0000-0000-0000000000aa",
 * });
 * ```
 */
export function buildNoteDoc(
  worldId: string,
  name: string | null,
  source: string,
  opts?: BuildNoteDocOptions,
): WireDocument {
  // `NoteEngine.sort` is typed `bigint` (ts-rs's i64 mapping), but `bigint` is
  // not JSON-serializable and this object is about to be `JSON.stringify`'d
  // onto the wire (`WsClient.send`) -- the same scalar gap `wire.ts`'s hand
  // mirror documents and resolves by using `number` for every i64 field that
  // actually crosses the wire. The real wire value is a plain JSON number
  // either way (`serde_json` serializes an i64 as a number), so this cast
  // states what is actually sent rather than the type ts-rs generated for it.
  const engine = {
    source,
    body: [],
    sort: opts?.sort ?? 0,
  } as unknown as NoteEngine;
  const doc = envelope(worldId, NOTE_DOC_TYPE, opts?.parentId ?? null, {}, opts?.id, engine, name);
  doc.permissions = { ...doc.permissions, default: "none" };
  if (opts?.owner) grantAuthor(doc, opts.owner);
  return doc;
}

/** Fail-closed read of a note's server-derived body: `null` for a non-note `doc_type` or a
 * malformed `engine.body` (a stored shape this client's schema cannot validate). Never returns
 * a partial list — the parse either succeeds for the whole array or fails closed.
 * @param doc The candidate document.
 * @returns The parsed segment list, or `null`.
 * @example
 * ```ts
 * import { parseNoteBody } from "@shadowcat/core";
 * import type { WireDocument } from "@shadowcat/core";
 *
 * declare const doc: WireDocument;
 * parseNoteBody(doc);
 * ```
 */
export function parseNoteBody(doc: WireDocument): (ChatSegment | UnknownSegment)[] | null {
  if (doc.doc_type !== NOTE_DOC_TYPE) return null;
  const engine = doc.engine as {
    /** The stored `NoteEngine.body`, of unvalidated shape until `SegmentListSchema` parses it. */
    body?: unknown;
  } | null | undefined;
  const r = SegmentListSchema.safeParse(engine?.body);
  return r.success ? r.data : null;
}
