// Client-owned scene-entity `system` shapes + pure document builders. The server
// stays structural-only (#6); these shapes are the client's interpretation of the
// opaque `system` body. Shared by WorldSession (scene auto-create) and the
// scene-tools module (token create) so the wire shape has one source of truth.
import type { WireDocument } from "./wire";

/** A scene's engine-owned config (M8d §15). Dimensions deferred (canvas pans freely). */
export interface SceneSystem {
  grid: { kind: "square" | "hex"; size: number };
  background: string | null;
}

/** A token's transform + visual (M8d §4). `(x,y)` is the token CENTER. `visual` is the
 * forward-looking seam — only `kind:"image"` ships in M8d. */
export interface TokenSystem {
  x: number;
  y: number;
  w: number;
  h: number;
  rotation: number;
  visual: { kind: "image"; asset: string };
}

/** Visible-to-all defaults; the server normalizes permissions per the creator's role. */
function defaultPermissions(): WireDocument["permissions"] {
  return { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} } };
}

function envelope(worldId: string, docType: string, parentId: string | null, system: unknown, id?: string): WireDocument {
  const now = Date.now();
  return {
    id: id ?? crypto.randomUUID(),
    scope: { kind: "world", world_id: worldId },
    doc_type: docType,
    schema_version: 1,
    source: null,
    owner: null,
    permissions: defaultPermissions(),
    embedded: {},
    parent_id: parentId,
    system,
    created_at: now,
    updated_at: now,
  };
}

/** A top-level scene document with a default square/100 grid and no background. */
export function buildSceneDoc(worldId: string, system: Partial<SceneSystem> = {}, id?: string): WireDocument {
  const full: SceneSystem = {
    grid: system.grid ?? { kind: "square", size: 100 },
    background: system.background ?? null,
  };
  return envelope(worldId, "scene", null, full, id);
}

/** A token document parented to `sceneId`, carrying the given transform + visual. */
export function buildTokenDoc(worldId: string, sceneId: string, system: TokenSystem, id?: string): WireDocument {
  return envelope(worldId, "token", sceneId, system, id);
}
