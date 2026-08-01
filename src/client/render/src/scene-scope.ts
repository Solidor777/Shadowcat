// Scene scoping for the render layer (M12d). The store holds EVERY scene's children (the server
// delivers the whole readable doc set); a client renders only the scene it is viewing. A `null`
// viewed scene (no scene exists yet) yields the unfiltered list — the degenerate pre-scene case,
// identical to legacy single-scene behavior.
import type { ReadableDocuments, WireDocument } from "@shadowcat/core";

/**
 * Filters `store`'s `docType` docs to the scene identified by `viewedSceneId()`. Each of
 * `TokenView`, `WallView`, `RegionView`, `TemplateView`, and `DrawingView` calls this
 * instead of `store.query(docType)` directly, so a client holding more than one scene's
 * documents renders only the one it is currently viewing.
 * @param store The document store to query.
 * @param docType The doc_type to fetch (e.g. `"token"`, `"wall"`).
 * @param viewedSceneId Resolves the currently-viewed scene id.
 * @returns `docType` docs whose `parent_id` equals the resolved scene id. When
 * `viewedSceneId()` resolves to `null` (the degenerate pre-scene case — no scene exists
 * yet, identical to legacy single-scene behavior), returns every `docType` doc unscoped.
 * @example
 * ```
 * // not exported from @shadowcat/render's index.ts; internal to the render-layer views
 * sceneScopedDocs(store, "token", () => "scene-a"); // only scene-a's tokens
 * sceneScopedDocs(store, "token", () => null); // every token doc in the store
 * ```
 */
export function sceneScopedDocs(
  store: ReadableDocuments,
  docType: string,
  viewedSceneId: () => string | null,
): WireDocument[] {
  const vsid = viewedSceneId();
  const docs = store.query(docType);
  return vsid === null ? docs : docs.filter((d) => d.parent_id === vsid);
}
