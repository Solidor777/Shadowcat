// The VFX playback seam: `VfxPlayRequest` is the wire-adjacent shape `AppContext.vfx.play`
// sends; `resolveVfxSource` is the ONE place a client turns an asset's metadata into
// something the render layer can draw, so `VfxView` and the FX scene tool never each grow
// their own resolution logic.
import type { Asset } from "@shadowcat/types";
import type { AssetResolver } from "./assets";

/** A request to play a VFX one-shot at a scene point, including `elevation?` — the render
 * layer filters by the viewed level. */
export interface VfxPlayRequest {
  /** Scene the effect plays on. */
  scene: string;
  /** The spritesheet or animated-source asset id. */
  asset: string;
  /** Scene-coordinate x. */
  x: number;
  /** Scene-coordinate y. */
  y: number;
  /** Uniform scale multiplier; omitted = the asset's native scale (1). */
  scale?: number;
  /** Rotation in degrees; omitted = unrotated. */
  rotation?: number;
  /** Playback duration cap in ms; omitted = one loop of the asset. */
  durationMs?: number;
  /** Paired sound asset id; carried through to `ServerMsg::Vfx` and played back through
   * `AudioApi.playOneShot` by the relaying client's Stage. */
  sound?: string;
  /** Elevation the effect plays at; the render layer filters by the viewed level. */
  elevation?: number;
}

/** A resolved, already-URL'd VFX source — the render package's `VfxNodeSpec.source` shape,
 * declared independently here (core cannot import a render-package type; the dependency runs
 * the other direction) but kept field-for-field identical so `VfxView` assigns a resolved
 * value straight in with no reshaping. See `@shadowcat/render`'s `ResolvedAnimatedSource`/
 * `ResolvedSheetSource` — any change to either shape must update both. */
export type ResolvedVfxSource =
  | {
      /** Discriminant: a server-derived grid sheet (`AssetMeta.sheet`). */
      type: "sheet";
      /** Sheet image serve URL. */
      url: string;
      /** Sheet row count. */
      rows: number;
      /** Sheet column count. */
      cols: number;
      /** Frame count to use, capped at `rows*cols`. */
      count?: number;
      /** Per-frame display duration in ms, in playback order. */
      frameMs?: number[];
    }
  | {
      /** Discriminant: a PixiJS-spritesheet-format pairing (`vfx:sheet=` tag). Deliberately
       * NOT `type: "sheet"` — see `ResolvedAnimatedSource`'s own arm, a different shape that
       * happens to share the word "sheet"; this member's own discriminant key is `kind`. */
      kind: "sheet";
      /** Atlas image serve URL. */
      imageUrl: string;
      /** Sidecar spritesheet-JSON serve URL. */
      sheetUrl: string;
      /** The animation to play — always `"default"`, the one animation name every paired
       * sidecar's `animations` map must define. */
      animation: string;
    };

/** Resolve an asset's VFX playback source: the server-derived grid sheet when
 * `meta.sheet` is present (takes priority — it needs no additional file resolution beyond
 * the asset's own serve URL), else a PixiJS spritesheet pairing when a `vfx:sheet=<id>` tag
 * is present, else `null` (fails closed — an asset that is neither is not a playable VFX
 * source, and the caller never plays anything for it).
 * @param meta The asset's metadata (from `AssetResolver`'s listing or `getAssetMeta`).
 * @param resolver Resolves asset ids to serve URLs.
 * @returns The resolved source, or `null`.
 * @example
 * ```ts
 * import { resolveVfxSource } from "@shadowcat/core";
 * import { AssetResolver } from "@shadowcat/core";
 * import type { Asset } from "@shadowcat/types";
 *
 * declare const meta: Asset;
 * resolveVfxSource(meta, new AssetResolver());
 * ```
 */
export function resolveVfxSource(meta: Asset, resolver: AssetResolver): ResolvedVfxSource | null {
  if (meta.sheet) {
    return {
      type: "sheet",
      // The `sheet` variant serves the derived tiled image (the canonical is the raw
      // animated source — slicing THAT as a grid would draw garbage).
      url: resolver.url(meta.id, "sheet"),
      rows: meta.sheet.rows,
      cols: meta.sheet.cols,
      count: meta.sheet.count,
      frameMs: meta.sheet.frame_ms,
    };
  }
  const sheetTag = meta.tags.find((t) => t.startsWith("vfx:sheet="));
  if (sheetTag) {
    const jsonId = sheetTag.slice("vfx:sheet=".length);
    if (!jsonId) return null;
    return {
      kind: "sheet",
      imageUrl: resolver.url(meta.id),
      sheetUrl: resolver.url(jsonId),
      animation: "default",
    };
  }
  return null;
}

/** A one-shot playback request carrying the server-broadcast `id` that keys this exact
 * playback in the render layer (`oneshot:<id>`) — the shape `VfxView.play`/
 * `RenderEngine.playVfx` take after a `ServerMsg::Vfx` arrives. Declared as a named type
 * rather than an inline `VfxPlayRequest & { id: string }` at each consumer, so the `id`
 * field is documented in exactly one place. */
export type VfxOneShotRequest = VfxPlayRequest & {
  /** Fresh per-broadcast id — the render layer's one-shot node key (`oneshot:<id>`). */
  id: string;
};
