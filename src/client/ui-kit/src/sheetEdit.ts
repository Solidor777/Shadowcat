import type { AppContext } from "./appContext";

/**
 * One field's change within a {@link setFields} batch.
 */
export interface FieldEdit {
  /** The field's JSON-pointer path within the document. */
  path: string;
  /** The real current stored value at `path` (OCC pre-image); `undefined` for a genuinely
   * absent field. */
  old: unknown;
  /** The new value to write. Ignored when {@link remove} is `true`. */
  value?: unknown;
  /** `true` to remove the object key at `path` instead of writing {@link value}, making it
   * GENUINELY ABSENT (`null` != absent) — mirrors `unsetField`'s standalone shape. */
  remove?: boolean;
}

/**
 * Dispatches ONE atomic Update carrying MULTIPLE `FieldChange`s — either all apply or none does.
 * The single builder of the `{op:"update", doc_id, changes}` envelope every sheet edit goes
 * through; {@link setField} and {@link unsetField} are thin single-edit wrappers over this.
 * INVARIANT (OCC): each edit's `old` must be the RAW current stored value at its `path`; the
 * server's `apply_intent` enforces field-level optimistic concurrency (`actual != change.old` →
 * Conflict), so a hardcoded or defaulted `old` is accepted only once and rejected+rolled-back on
 * every subsequent edit. `old ?? null` collapses ONLY a genuinely absent (`undefined`) pre-image
 * to the wire's `null` — a falsy real value (`0`/`false`/`""`) is preserved verbatim. Use this
 * directly whenever two or more fields of one document must move together as a single
 * post-image (e.g. a table's `draw` rule and its `rows` array, which the server validates
 * jointly) — a sheet must never fork that into two separate single-field dispatches, since the
 * server could then accept one write and reject the other, leaving an inconsistent post-image
 * the sheet never intended.
 * @param ctx - The AppContext to dispatch the intent through.
 * @param docId - The target document's id.
 * @param edits - The fields to change, all as one Update.
 * @example setFields(ctx, docId, [{ path: "/engine/draw", old, value: next }]);
 */
export function setFields(ctx: AppContext, docId: string, edits: FieldEdit[]): void {
  ctx.dispatchIntent([
    {
      op: "update",
      doc_id: docId,
      changes: edits.map(({ path, old, value, remove }) =>
        remove ? { path, old: old ?? null, new: null, remove: true } : { path, old: old ?? null, new: value },
      ),
    },
  ]);
}

/**
 * The single field-path Update dispatch for every sheet. Thin wrapper over {@link setFields}
 * for the common single-edit case; see {@link setFields} for the OCC invariant.
 * @param ctx - The AppContext to dispatch the intent through.
 * @param docId - The target document's id.
 * @param path - The field's JSON-pointer path within the document.
 * @param old - The real current stored value at `path` (OCC pre-image); `undefined` for
 * a genuinely absent field.
 * @param value - The new value to write.
 * @example setField(ctx, docId, "/system/hp", currentHp, 12);
 */
export function setField(ctx: AppContext, docId: string, path: string, old: unknown, value: unknown): void {
  setFields(ctx, docId, [{ path, old, value }]);
}

/**
 * Remove the object key at `path`, making it GENUINELY ABSENT (`null` != absent). Thin wrapper
 * over {@link setFields} for the common single-edit removal case; see {@link setFields} for the
 * OCC invariant. Server-side `remove_pointer` handles object keys only — array-element removal
 * still goes through `setField` with a whole-array replacement.
 * @param ctx - The AppContext to dispatch the intent through.
 * @param docId - The target document's id.
 * @param path - The object key's JSON-pointer path within the document.
 * @param old - The real current stored value at `path` (OCC pre-image); `undefined` for
 * a genuinely absent field.
 * @example unsetField(ctx, docId, "/system/tempFlag", currentValue);
 */
export function unsetField(ctx: AppContext, docId: string, path: string, old: unknown): void {
  setFields(ctx, docId, [{ path, old, remove: true }]);
}
