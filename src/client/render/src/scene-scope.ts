// Scene scoping for the render layer (M12d). The store holds EVERY scene's children (the server
// delivers the whole readable doc set); a client renders only the scene it is viewing. A `null`
// viewed scene (no scene exists yet) yields the unfiltered list — the degenerate pre-scene case,
// identical to legacy single-scene behavior.
import type { ReadableDocuments, WireDocument } from "@shadowcat/core";

export function sceneScopedDocs(
  store: ReadableDocuments,
  docType: string,
  viewedSceneId: () => string | null,
): WireDocument[] {
  const vsid = viewedSceneId();
  const docs = store.query(docType);
  return vsid === null ? docs : docs.filter((d) => d.parent_id === vsid);
}
