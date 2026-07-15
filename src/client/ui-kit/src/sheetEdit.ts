// The single field-path Update dispatch for every sheet (M12c). INVARIANT (OCC): `old`
// is the RAW current stored value at `path`; the server's apply_intent enforces
// field-level optimistic concurrency (actual != change.old -> Conflict), so a hardcoded
// or defaulted `old` is accepted only once and rejected+rolled-back on every subsequent
// edit (the M11d-2 GameSettingsPanel Critical). `old ?? null` collapses ONLY a genuinely
// absent (undefined) pre-image to the wire's null — a falsy real value (0/false/"") is
// preserved verbatim.
import type { AppContext } from "./appContext";

export function setField(ctx: AppContext, docId: string, path: string, old: unknown, value: unknown): void {
  ctx.dispatchIntent([{ op: "update", doc_id: docId, changes: [{ path, old: old ?? null, new: value }] }]);
}
