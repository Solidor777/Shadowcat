import { DocumentStore, buildSceneDoc, buildTokenDoc, type WireDocument } from "@shadowcat/core";

/** Options for {@link messageDoc}. */
export interface MessageOptions {
  /** The message kind; defaults to `"normal"`. */
  kind?: string;
  /** Who the message is spoken as; absent means no attribution. */
  actorOwner?: unknown;
  /** The soft-delete marker. */
  deletedAt?: number;
  /** The text of the single content segment. */
  text?: string;
}

/**
 * A message document with one text segment.
 * @param id The document id.
 * @param over Kind, attribution, delete marker and text overrides.
 * @returns The message document.
 * @example
 * ```ts
 * messageDoc("m1", { kind: "emote", text: "waves" });
 * ```
 */
export function messageDoc(id: string, over: MessageOptions = {}): WireDocument {
  return {
    id, scope: { kind: "world", world_id: "w1" }, doc_type: "message", schema_version: 1,
    name: null, source: null, owner: "u1",
    permissions: { default: "observer", users: {} } as WireDocument["permissions"],
    embedded: {}, parent_id: null,
    engine: {
      channel: "general", user_owner: "u1", kind: over.kind ?? "normal", audience: { kind: "public" },
      actor_owner: over.actorOwner ?? null,
      deleted_at: over.deletedAt,
      content: [{ kind: "text", text: over.text ?? "hello" }],
    },
    system: {}, created_at: 1, updated_at: 1,
  };
}

/**
 * A token document with a 100 x 100 footprint.
 * @param id The document id.
 * @param sceneId The scene the token is placed on.
 * @param x Scene-pixel x of the token centre.
 * @param y Scene-pixel y of the token centre.
 * @param actorId The linked actor id, or `null`.
 * @returns The token document.
 * @example
 * ```ts
 * tokenDoc("t1", "s1", 500, 400, "a1");
 * ```
 */
export function tokenDoc(id: string, sceneId: string, x: number, y: number, actorId: string | null = null): WireDocument {
  return buildTokenDoc(
    "w1",
    sceneId,
    { x, y, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: actorId, overrides: null, face: null, elevation: null, lock_rotation: false, identity_hidden: false },
    id,
  );
}

/**
 * A store holding scene `sceneId` plus `docs`, all created in one command.
 * @param sceneId The scene's document id.
 * @param docs Further documents to create.
 * @returns The populated store.
 * @example
 * ```ts
 * storeWith("s1", tokenDoc("t1", "s1", 0, 0));
 * ```
 */
export function storeWith(sceneId: string, ...docs: WireDocument[]): DocumentStore {
  const store = new DocumentStore();
  store.applyCommand({
    seq: 1, world_id: "w1", author: "a", ts: 0,
    ops: [buildSceneDoc("w1", { grid: { kind: "square", size: 100, distance: null } }, sceneId), ...docs].map((doc) => ({ op: "create" as const, doc })),
  } as never);
  return store;
}

/**
 * Applies `docs` to `store` as one later command.
 * @param store The store to write to.
 * @param seq The command's sequence number; must follow the last applied one.
 * @param docs The documents to create.
 * @example
 * ```ts
 * addDocs(storeWith("s1"), 2, messageDoc("m1"));
 * ```
 */
export function addDocs(store: DocumentStore, seq: number, ...docs: WireDocument[]): void {
  store.applyCommand({ seq, world_id: "w1", author: "a", ts: 0, ops: docs.map((doc) => ({ op: "create" as const, doc })) } as never);
}
