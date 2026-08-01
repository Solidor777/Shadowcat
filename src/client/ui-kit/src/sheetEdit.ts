import type { AppContext } from "./appContext";

/**
 * The single field-path Update dispatch for every sheet (M12c). INVARIANT (OCC): `old`
 * must be the RAW current stored value at `path`; the server's `apply_intent` enforces
 * field-level optimistic concurrency (`actual != change.old` → Conflict), so a hardcoded
 * or defaulted `old` is accepted only once and rejected+rolled-back on every subsequent
 * edit (the M11d-2 GameSettingsPanel Critical). `old ?? null` collapses ONLY a genuinely
 * absent (`undefined`) pre-image to the wire's `null` — a falsy real value (`0`/`false`/`""`)
 * is preserved verbatim.
 * @param ctx - The AppContext to dispatch the intent through.
 * @param docId - The target document's id.
 * @param path - The field's JSON-pointer path within the document.
 * @param old - The real current stored value at `path` (OCC pre-image); `undefined` for
 * a genuinely absent field.
 * @param value - The new value to write.
 * @example setField(ctx, docId, "/system/hp", currentHp, 12);
 */
export function setField(ctx: AppContext, docId: string, path: string, old: unknown, value: unknown): void {
  ctx.dispatchIntent([{ op: "update", doc_id: docId, changes: [{ path, old: old ?? null, new: value }] }]);
}

/**
 * Remove the object key at `path`, making it GENUINELY ABSENT (`null` != absent).
 * `old` is the OCC pre-image of the value being removed (same INVARIANT as `setField`).
 * Server-side `remove_pointer` handles object keys only — array-element removal still
 * goes through `setField` with a whole-array replacement.
 * @param ctx - The AppContext to dispatch the intent through.
 * @param docId - The target document's id.
 * @param path - The object key's JSON-pointer path within the document.
 * @param old - The real current stored value at `path` (OCC pre-image); `undefined` for
 * a genuinely absent field.
 * @example unsetField(ctx, docId, "/system/tempFlag", currentValue);
 */
export function unsetField(ctx: AppContext, docId: string, path: string, old: unknown): void {
  ctx.dispatchIntent([{ op: "update", doc_id: docId, changes: [{ path, old: old ?? null, new: null, remove: true }] }]);
}
