// Scene scoping for the render layer. The store holds EVERY scene's children (the server
// delivers the whole readable doc set); a client renders only the scene it is viewing. A `null`
// viewed scene (no scene exists yet) yields the unfiltered list — the degenerate pre-scene case,
// identical to legacy single-scene behavior.
import type { ReadableDocuments, WireDocument, SceneEngine, ElevationBand } from "@shadowcat/core";
import { bandContains, levelOf } from "@shadowcat/core";

/** `docType`s whose engine body carries a BAND (`Option<ElevationBand>` at `engine.elevation`) —
 * scoped to a viewed level by `bandContains` at the LEVEL'S OWN `bottom`, mirroring
 * `scene::elevation::band_contains`'s wall-occlusion convention. Every other scoped doc type
 * (currently only `"token"`) carries a POINT elevation and is scoped by `levelOf` instead. */
const BAND_SHAPED_DOC_TYPES = new Set(["wall", "region", "drawing", "template"]);

/** The `elevation` shape a band-shaped doc's `engine` body carries, as far as level scoping
 * cares — a structural subset of `WallEngine`/`RegionEngine`/`DrawingEngine`/`TemplateEngine`. */
interface BandShapedEngine {
  /** The document's elevation band; absent/`null` means "every level". */
  elevation?: ElevationBand | null;
}

/** The `elevation` shape a point-elevation doc's `engine` body carries, as far as level scoping
 * cares — a structural subset of `TokenEngine`/`LightEngine`. */
interface PointElevationEngine {
  /** The document's point elevation; absent/`null` means ground (`0`). */
  elevation?: number | null;
}

/**
 * Filters `store`'s `docType` docs to the scene identified by `viewedSceneId()`, then — when
 * `viewedLevel()` resolves to a level present on that scene's `SceneEngine.levels` — additionally
 * to that level. Each of `TokenView`, `WallView`, `RegionView`, `TemplateView`, `DrawingView` and
 * `LightView` calls this instead of `store.query(docType)` directly, so a client holding more
 * than one scene's (or one scene's multiple levels') documents renders only what it is currently
 * viewing.
 * @param store The document store to query.
 * @param docType The doc_type to fetch (e.g. `"token"`, `"wall"`).
 * @param viewedSceneId Resolves the currently-viewed scene id.
 * @param viewedLevel Resolves the currently-viewed level id; `null` (default) means "every
 * level" — the degenerate pre-levels case, preserving today's behavior exactly for a scene whose
 * `levels` is empty.
 * @returns `docType` docs whose `parent_id` equals the resolved scene id, additionally filtered
 * to the resolved level when one applies. When `viewedSceneId()` resolves to `null` (the
 * degenerate pre-scene case — no scene exists yet, identical to legacy single-scene behavior),
 * returns every `docType` doc unscoped (the level filter is skipped in that case too, since there
 * is no scene to read `levels` from).
 * @example
 * ```
 * // not exported from @shadowcat/render; internal to the render-layer views
 * declare const store: ReadableDocuments;
 * sceneScopedDocs(store, "token", () => "scene-a"); // only scene-a's tokens
 * sceneScopedDocs(store, "token", () => null); // every token doc in the store
 * sceneScopedDocs(store, "token", () => "scene-a", () => "l1"); // scene-a's tokens on level l1
 * ```
 */
export function sceneScopedDocs(
  store: ReadableDocuments,
  docType: string,
  viewedSceneId: () => string | null,
  viewedLevel: () => string | null = () => null,
): WireDocument[] {
  const vsid = viewedSceneId();
  const docs = store.query(docType);
  const sceneScoped = vsid === null ? docs : docs.filter((d) => d.parent_id === vsid);
  const level = viewedLevel();
  if (level === null || vsid === null) return sceneScoped;
  const sceneDoc = store.query("scene").find((s) => s.id === vsid);
  const levels = (sceneDoc?.engine as SceneEngine | undefined)?.levels ?? [];
  if (levels.length === 0) return sceneScoped;
  const target = levels.find((l) => l.id === level);
  if (!target) return sceneScoped;
  if (BAND_SHAPED_DOC_TYPES.has(docType)) {
    return sceneScoped.filter((d) =>
      bandContains((d.engine as BandShapedEngine | undefined)?.elevation ?? null, target.bottom),
    );
  }
  return sceneScoped.filter(
    (d) => levelOf(levels, (d.engine as PointElevationEngine | undefined)?.elevation ?? 0)?.id === level,
  );
}
