// Client mirror of the `playlist`/`audio-state` engine documents
// (`data::engine::audio::{PlaylistEngine, AudioStateEngine}`). Both ARE ts-rs-exported (cross
// the wire boundary as stored documents) — this module re-exports the generated types verbatim
// and adds only the client-side construction helper for `playlist` (nothing ever client-
// constructs an `audio-state` document — see `WriteOrigin::AudioTransport`'s guard).
import { envelope, grantAuthor } from "./scene-docs";
import type { PlaylistEngine } from "@shadowcat/types";
import type { WireDocument } from "./wire";

export type {
  AudioChannel,
  AudioStateEngine,
  PlayingTrack,
  PlaylistEngine,
  PlaylistMode,
  PlaylistTrack,
} from "@shadowcat/types";

/** The `doc_type` identifying a stored playlist document (server:
 * `data::engine::audio::PLAYLIST_DOC_TYPE`). */
export const PLAYLIST_DOC_TYPE = "playlist";
/** The `doc_type` identifying the world's singleton audio-transport-state document (server:
 * `data::engine::audio::AUDIO_STATE_DOC_TYPE`). Never client-constructed — see this doc's own
 * module comment. */
export const AUDIO_STATE_DOC_TYPE = "audio-state";

/** Optional construction parameters for `buildPlaylistDoc`. */
export interface BuildPlaylistDocOptions {
  /** Optional explicit document id; a fresh uuid is generated when omitted. */
  id?: string;
  /** The authoring user's id. When given, grants that user `Owner` PLUS `AUTHOR_CAPS` via
   * `grantAuthor` — same rationale as `buildTableDoc`'s `owner` option. */
  owner?: string;
}

/** Builds an unsaved `playlist` document: standalone, `permissions.default: "observer"`
 * (readable by every world member by default, same as `table`/`note`), `system: {}`.
 * @param worldId The owning world's id.
 * @param name The playlist's display name (envelope `name`).
 * @param engine The playlist's engine body.
 * @param opts Optional explicit id and authoring owner.
 * @returns The unsaved `WireDocument`, ready to `Create`.
 * @example
 * ```ts
 * import { buildPlaylistDoc } from "@shadowcat/core";
 *
 * buildPlaylistDoc("00000000-0000-0000-0000-000000000001", "Tavern", {
 *   tracks: [],
 *   mode: "sequential",
 *   channel: "music",
 *   fadeMs: 2000,
 * }, { owner: "00000000-0000-0000-0000-0000000000aa" });
 * ```
 */
export function buildPlaylistDoc(
  worldId: string,
  name: string,
  engine: PlaylistEngine,
  opts?: BuildPlaylistDocOptions,
): WireDocument {
  const doc = envelope(worldId, PLAYLIST_DOC_TYPE, null, {}, opts?.id, engine, name);
  if (opts?.owner) grantAuthor(doc, opts.owner);
  return doc;
}
