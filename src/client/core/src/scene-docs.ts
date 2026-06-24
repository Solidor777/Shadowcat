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
  /** Set on raw (actorless) tokens; actor-backed tokens resolve their visual via the actor. */
  visual?: { kind: "image"; asset: string };
  /** Linked token: the shared actor's id (null/absent ⇒ instanced, see `embedded.actor`). */
  actor_id?: string | null;
  /** Linked-only per-token override whitelist (name/visual/size). */
  overrides?: TokenOverrides;
}

/** An actor's appearance + defaults (M10a). Stats/sheet are M12; this is only what backs a
 * token. The server is structural-only — this `system` shape is the client's interpretation. */
export interface ActorVisual {
  kind: "image";
  asset: string;
}
export interface ActorSystem {
  name: string;
  displayName: string;
  visual: ActorVisual;
  size: { w: number; h: number };
  shape: "square" | "circle";
  faction: string | null;
  conditions: string[];
  /** Default place-mode: true ⇒ instance (independent copy) on drop; false ⇒ link (shared). */
  prototype: boolean;
}

/** The per-token override whitelist for a linked token (M10a). */
export interface TokenOverrides {
  name?: string;
  visual?: ActorVisual;
  size?: { w: number; h: number };
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

/** A top-level (world-scoped, parentless) actor document. */
export function buildActorDoc(worldId: string, system: ActorSystem, id?: string): WireDocument {
  return envelope(worldId, "actor", null, system, id);
}

/** Build a token from an actor. `link` references the shared actor; `instance` embeds an
 * independent copy with `source` provenance (the deferred merge engine consumes it). Size/
 * shape resolve from the actor (M10d); `w`/`h` seed the rendered cell size now. */
export function buildTokenFromActor(
  worldId: string,
  sceneId: string,
  actor: WireDocument,
  mode: "link" | "instance",
  pos: { x: number; y: number },
  cellSize: number,
  id?: string,
): WireDocument {
  const base: TokenSystem = { x: pos.x, y: pos.y, w: cellSize, h: cellSize, rotation: 0 };
  if (mode === "link") {
    return envelope(worldId, "token", sceneId, { ...base, actor_id: actor.id, overrides: {} }, id);
  }
  // Deep-clone so the embedded copy is independent by value at construction (not just after
  // the wire round-trip) — no aliasing of the source actor's system/permissions/embedded.
  const copy: WireDocument = { ...structuredClone(actor), id: crypto.randomUUID(), source: { id: actor.id, pack: null, version: 1 } };
  const doc = envelope(worldId, "token", sceneId, base, id);
  doc.embedded = { actor: [copy] };
  return doc;
}

/** Set/clear the name-privacy override on an actor doc's permissions: hiding declares
 * `/system/name` as the `owner_or_gm` tier (the server redacts it from non-owner players on
 * egress, and retroactively retracts an already-delivered value when the override is added);
 * clearing removes the declaration. Mutates in place + returns `doc`. */
export function setNameHidden(doc: WireDocument, hidden: boolean): WireDocument {
  const overrides = { ...doc.permissions.property_overrides };
  if (hidden) overrides["/system/name"] = "owner_or_gm";
  else delete overrides["/system/name"];
  doc.permissions = { ...doc.permissions, property_overrides: overrides };
  return doc;
}

/** A token document parented to `sceneId`, carrying the given transform + visual. */
export function buildTokenDoc(worldId: string, sceneId: string, system: TokenSystem, id?: string): WireDocument {
  return envelope(worldId, "token", sceneId, system, id);
}

/** A generic scene-entity document (drawing/template/…) parented to `sceneId`; the
 * `system` shape is the caller's (client-owned, server structural-only). */
export function buildSceneEntityDoc(worldId: string, sceneId: string, docType: string, system: unknown, id?: string): WireDocument {
  return envelope(worldId, docType, sceneId, system, id);
}
