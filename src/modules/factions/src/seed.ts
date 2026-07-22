import { buildFactionRegistryDoc, deterministicId, type Faction, type ReadableDocuments, type WireOperation } from "@shadowcat/core";

/** Default three-faction seed content. Two GMs racing to seed a brand-new world dispatch
 * Creates that share the SAME deterministic id — not byte-identical content, since `envelope()`
 * stamps `created_at`/`updated_at` via `Date.now()` per call — so the server's singleton
 * create-gate (doc_type-scoped, not id-scoped) rejects the loser, and because both used the
 * same id the loser's rolled-back optimistic prediction and the winner's confirmed doc share
 * one store key — there is never a visible second registry to reconcile away. */
export const SEED: Record<string, Faction> = {
  friendly: { name: "Friendly", color: "#3fb950", stance: "friendly" },
  neutral: { name: "Neutral", color: "#9e9e9e", stance: "neutral" },
  hostile: { name: "Hostile", color: "#f85149", stance: "hostile" },
};

/** Idempotent GM seed: creates the world's `faction-registry` singleton under a deterministic
 * id (derived from `worldId`, so every client computes the same id without a lookup) only when
 * it is absent from `store`. A no-op if the doc already exists — including the case where
 * this client lost a create race: the server rejected its Create (`DataError::Conflict`), the
 * optimistic layer rolled the local prediction back automatically (`WsClient.onReject` ->
 * `OptimisticClient.reject`), and the winner's doc later arrives under the same deterministic
 * id via the normal event/resync stream. No explicit conflict-catching is needed or possible
 * here: `dispatchIntent` is fire-and-forget (AppContext exposes no per-call reject signal to
 * modules) by design (see `ChatApi`'s identical fire-and-forget contract). */
export function seedFactionRegistryIfAbsent(
  store: ReadableDocuments,
  worldId: string,
  dispatchIntent: (ops: WireOperation[]) => void,
): void {
  const id = deterministicId(worldId, "faction-registry");
  if (store.get(id) || store.query("faction-registry").length > 0) return;
  dispatchIntent([{ op: "create", doc: buildFactionRegistryDoc(worldId, SEED, id) }]);
}
