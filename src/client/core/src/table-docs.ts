// Client mirror of the `table` engine document (`data::engine::table::TableEngine`).
// Unlike chat's `MessageEngine`, `TableEngine` IS ts-rs-exported (crosses the wire boundary
// as a stored, editable document) — this module re-exports the generated types verbatim
// and adds only the client-side construction helper.
import { envelope } from "./scene-docs";
import type { TableEngine } from "@shadowcat/types";
import type { WireDocument } from "./wire";

export type { TableEngine, DrawRule, TableRow, RowRange, TableEntry } from "@shadowcat/types";

/** The `doc_type` identifying a stored rollable-table document (server:
 * `data::engine::table::TABLE_DOC_TYPE`). */
export const TABLE_DOC_TYPE = "table";

/** Builds an unsaved `table` document: standalone (never embedded, never
 * parented — see `data::validation::validate_containment`'s `table` arm),
 * `permissions.default: "observer"` (readable by every world member by
 * default, same as an actor), `system: {}` (a table has no opaque band).
 * @param worldId The owning world's id.
 * @param name The table's display name (envelope `name`).
 * @param engine The table's engine body.
 * @param id Optional explicit document id; a fresh uuid is generated when omitted.
 * @returns The unsaved `WireDocument`, ready to `Create`.
 * @example
 * ```ts
 * import { buildTableDoc } from "@shadowcat/core";
 *
 * buildTableDoc("00000000-0000-0000-0000-000000000001", "Loot", {
 *   draw: { kind: "weighted" },
 *   rows: [{ weight: 1, range: null, label: "a sword", results: [] }],
 *   description: "",
 * });
 * ```
 */
export function buildTableDoc(worldId: string, name: string, engine: TableEngine, id?: string): WireDocument {
  return envelope(worldId, TABLE_DOC_TYPE, null, {}, id, engine, name);
}
