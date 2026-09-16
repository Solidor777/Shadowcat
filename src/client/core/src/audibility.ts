import { z } from "zod";

/** One spatially-resolved carried sound emitter — the client mirror of the generated
 * `AudibleEmitter`. Every reduction (falloff, occlusion, authored volume) is already folded
 * into `gain`; there is no client-side distance/occlusion formula. */
export interface AudibleEmitter {
  /** The carrying token's id. */
  token: string;
  /** Asset id to play. */
  asset: string;
  /** Final resolved gain, `0..=1`. */
  gain: number;
  /** Stereo pan, `-1..=1`. */
  pan: number;
  /** Loop the asset. */
  loop: boolean;
}

/** One scene's resolved slice of the `"audibility"` derived channel — the client mirror of the
 * generated `SceneAudibility`. A recipient subscribed to (or a GM locally roaming across) more
 * than one scene receives one of these per scene in the SAME payload; `sceneAudibility` picks
 * the slice for the scene being viewed. */
export interface SceneAudibility {
  /** The scene this slice belongs to. */
  scene: string;
  /** The recipient's resolved listening token in `scene`, or `null`. */
  listener: string | null;
  /** Whether spatial attenuation/occlusion/pan are active in `scene`. */
  spatial: boolean;
  /** Every audible carried emitter of `scene`, already spatially resolved. */
  emitters: AudibleEmitter[];
}

/** The `"audibility"` derived channel's full payload — the client mirror of the generated
 * `AudibilityPayload`. One `SceneAudibility` per visible scene with at least one token,
 * mirroring `FootprintsPayload`'s own `scenes: [...]` shape. */
export interface AudibilityPayload {
  /** One entry per visible scene with at least one token. */
  scenes: SceneAudibility[];
}

/** Wire shape of one scene's audibility slice. */
const sceneAudibilitySchema = z.object({
  scene: z.string(),
  listener: z.string().nullable(),
  spatial: z.boolean(),
  emitters: z.array(
    z.object({
      token: z.string(),
      asset: z.string(),
      gain: z.number().finite(),
      pan: z.number().finite(),
      loop: z.boolean(),
    }),
  ),
});

/** Wire shape of the `"audibility"` derived channel payload — the Zod mirror of the generated
 * `AudibilityPayload`. */
const payloadSchema = z.object({ scenes: z.array(sceneAudibilitySchema) });

/** The state before the first `"audibility"` frame arrives: no scenes at all. */
export const EMPTY_AUDIBILITY: AudibilityPayload = { scenes: [] };

/** A single scene's empty slice: no listener, spatial on (the engine-literal default), nothing
 * audible. Returned by `sceneAudibility` for a scene absent from the payload (no tokens in it,
 * or no frame has arrived yet) and for a `null` viewed-scene id. */
export const EMPTY_SCENE_AUDIBILITY: SceneAudibility = {
  scene: "",
  listener: null,
  spatial: true,
  emitters: [],
};

/**
 * Parse a `"audibility"` derived-channel payload.
 *
 * A payload that does not validate yields {@link EMPTY_AUDIBILITY} (silence) rather than a
 * partial read, matching `parseFootprints`'s own fail-closed rationale: every carried emitter
 * here is a live Web Audio node about to start playing, and a half-parsed set could leave one
 * running with a garbled gain/pan the caller has no way to distinguish from an authoritative
 * (if unusual) server value.
 * @param payload The raw `SceneDerived` payload for the `"audibility"` channel.
 * @returns The parsed payload, or {@link EMPTY_AUDIBILITY} when it does not validate.
 * @example
 * ```ts
 * import { parseAudibility } from "@shadowcat/core";
 *
 * declare const payload: unknown;
 * const audibility = parseAudibility(payload);
 * audibility.scenes; // SceneAudibility[]
 * ```
 */
export function parseAudibility(payload: unknown): AudibilityPayload {
  const parsed = payloadSchema.safeParse(payload);
  return parsed.success ? parsed.data : EMPTY_AUDIBILITY;
}

/**
 * Picks the slice for `sceneId` out of a full multi-scene payload — the client's own filter to
 * the scene it is viewing, mirroring how `"vision"`/`"footprints"` consumers already select a
 * scene out of their own multi-scene payloads.
 * @param payload The full `"audibility"` payload.
 * @param sceneId The scene currently being viewed (`WorldSession.viewedSceneId`), or `null`.
 * @returns The matching slice, or {@link EMPTY_SCENE_AUDIBILITY} when absent.
 * @example
 * ```ts
 * import { parseAudibility, sceneAudibility } from "@shadowcat/core";
 *
 * declare const payload: unknown;
 * const slice = sceneAudibility(parseAudibility(payload), "scene-1");
 * slice.emitters; // AudibleEmitter[]
 * ```
 */
export function sceneAudibility(payload: AudibilityPayload, sceneId: string | null): SceneAudibility {
  if (sceneId === null) return EMPTY_SCENE_AUDIBILITY;
  return payload.scenes.find((s) => s.scene === sceneId) ?? EMPTY_SCENE_AUDIBILITY;
}
