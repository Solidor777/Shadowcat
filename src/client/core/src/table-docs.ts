// Client mirror of the `table` engine document (`data::engine::table::TableEngine`).
// Unlike chat's `MessageEngine`, `TableEngine` IS ts-rs-exported (crosses the wire boundary
// as a stored, editable document) — this module re-exports the generated types verbatim
// and adds only the client-side construction helper.
import { envelope, grantAuthor } from "./scene-docs";
import type { TableEngine } from "@shadowcat/types";
import type { WireDocument } from "./wire";

export type { TableEngine, DrawRule, TableRow, RowRange, TableEntry } from "@shadowcat/types";

/** The `doc_type` identifying a stored rollable-table document (server:
 * `data::engine::table::TABLE_DOC_TYPE`). */
export const TABLE_DOC_TYPE = "table";

/** Optional construction parameters for `buildTableDoc`. */
export interface BuildTableDocOptions {
  /** Optional explicit document id; a fresh uuid is generated when omitted. */
  id?: string;
  /** The authoring user's id. When given, grants that user `Owner` PLUS `AUTHOR_CAPS`
   * (`core:delete`, `core:edit_permissions`) via `grantAuthor` — the `owner` DocRole floor
   * alone cannot delete the table or reshare it. Omitting it leaves the table's
   * `permissions.default: "observer"` floor as the only access (every world member reads,
   * nobody but a GM may delete or reshare). */
  owner?: string;
}

/** Builds an unsaved `table` document: standalone (never embedded, never
 * parented — see `data::validation::validate_containment`'s `table` arm),
 * `permissions.default: "observer"` (readable by every world member by
 * default, same as an actor), `system: {}` (a table has no opaque band).
 * @param worldId The owning world's id.
 * @param name The table's display name (envelope `name`).
 * @param engine The table's engine body.
 * @param opts Optional explicit id and authoring owner.
 * @returns The unsaved `WireDocument`, ready to `Create`.
 * @example
 * ```ts
 * import { buildTableDoc } from "@shadowcat/core";
 *
 * buildTableDoc("00000000-0000-0000-0000-000000000001", "Loot", {
 *   draw: { kind: "weighted" },
 *   rows: [{ weight: 1, range: null, label: "a sword", results: [] }],
 *   description: "",
 * }, { owner: "00000000-0000-0000-0000-0000000000aa" });
 * ```
 */
export function buildTableDoc(
  worldId: string,
  name: string,
  engine: TableEngine,
  opts?: BuildTableDocOptions,
): WireDocument {
  const doc = envelope(worldId, TABLE_DOC_TYPE, null, {}, opts?.id, engine, name);
  if (opts?.owner) grantAuthor(doc, opts.owner);
  return doc;
}
