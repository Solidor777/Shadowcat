// The combat seam: reads over the per-recipient optimistic view, server-resolved resource
// numbers over the "combat" derived channel, the server-owned clock's intents, and document
// helpers that build/mutate combat/combatant documents through ordinary intents. The client
// evaluates and stores no combat formula -- every resolved number comes from the "combat"
// channel (parseCombats).
import type { ReadableDocuments } from "./store";
import type { WireDocument, ClientMsg, WireCombatRollEntry, WireResourceOp, WireOperation } from "./wire";
import type { CombatsView, CombatantView } from "./wire";
import { EMPTY_COMBATS } from "./wire";
import type { CombatEngine, CombatantEngine, CombatantKind } from "./scene-docs";
import { buildCombatDoc, buildCombatantDoc, newCombatEngine } from "./scene-docs";
import { effectiveOwner } from "./actor";
import type { Logger } from "./logger";

/** The host-provided service id `CombatController` registers under
 * (`ServiceRegistry.provide`/`ModuleContext.services.get`). */
export const COMBAT_SERVICE = "shadowcat.service:combat";

/** A world role, as resolved by `WorldSession.role` -- `null` before `Welcome` arrives. */
export type WorldRole = "gm" | "player";

/** Options for `CombatApi.createCombat`. */
export interface CreateCombatOptions {
  /** Display name; `null`/omitted leaves the combat unnamed. */
  name?: string | null;
  /** Explicit document id, or omitted to generate one. */
  id?: string;
}

/** A new combatant to add via `CombatApi.addCombatants`. At least one of `tokenId`/`actorId`
 * must resolve to a document, or the call throws `CombatClientError` (`code: "no-host"`). */
export interface NewCombatant {
  /** The token this combatant represents. */
  tokenId?: string;
  /** The actor this combatant represents (explicit override of the token's own actor link). */
  actorId?: string;
  /** Hidden = unreadable to every non-GM. */
  hidden?: boolean;
  /** Display name override; defaults to the token's, else the actor's. */
  name?: string | null;
  /** Opaque system body. */
  system?: unknown;
}

/** A new one-shot event to add via `CombatApi.addEvent`. */
export interface NewEvent {
  /** Display name. */
  name: string;
  /** Turns remaining before the event auto-resolves; `null` = indefinite. */
  lifespan: number | null;
  /** Flavor text shown when the event's turn comes up. */
  message: string | null;
  /** Hidden = unreadable to every non-GM. */
  hidden?: boolean;
  /** Opaque system body. */
  system?: unknown;
}

/** Advisory UI-gating affordances for one combat -- the server is the sole authority; every
 * flag here mirrors `combat::authorize` and may be wrong for one round-trip after a permission
 * change the client has not yet observed. */
export interface CombatAffordances {
  /** GM-only: `combat_start`. Additionally requires a non-empty `order`. */
  start: boolean;
  /** GM-only: `combat_pause`. */
  pause: boolean;
  /** GM-only: `combat_end`. */
  end: boolean;
  /** GM, or the current turn's owner under `owner_may_end`: `combat_advance`. */
  advance: boolean;
  /** GM-only, and only once `round > 0`: `combat_rewind`. */
  rewind: boolean;
  /** GM-only: `combat_sort`. */
  sort: boolean;
  /** GM-only: add/remove/reorder/hide/delete a combatant. */
  edit: boolean;
  /** Whether `combatantId`'s owner may roll/act for it: GM, or its owner with `canEdit(doc,
   * "/engine")`.
   * @param combatantId The combatant to check.
   * @returns Whether the caller may roll for this combatant. */
  roll(combatantId: string): boolean;
  /** Same rule as `roll` -- whether the caller may spend this combatant's resources.
   * @param combatantId The combatant to check.
   * @returns Whether the caller may adjust this combatant's resources. */
  resource(combatantId: string): boolean;
}

/** An error `CombatApi`'s document helpers throw for a client-side rule violation (never a
 * server rejection, which surfaces as a rejected `Promise` from an intent method instead). */
export class CombatClientError extends Error {
  /** Which client-side rule was violated. */
  code: "no-host" | "turn-owner" | "order-mismatch" | "not-found";
  /**
   * @param code Which rule was violated.
   * @param message Human-readable detail.
   * @example
   * ```ts
   * import { CombatClientError } from "@shadowcat/core";
   *
   * throw new CombatClientError("no-host", "a combatant needs a token or an actor");
   * ```
   */
  constructor(code: CombatClientError["code"], message: string) {
    super(message);
    this.code = code;
    this.name = "CombatClientError";
  }
}

/** Dependencies `CombatController` is constructed with -- the host wires these from its own
 * `WsClient`/`DocumentStore`/session state. */
export interface CombatControllerDeps {
  /** The optimistic per-recipient document view -- every read goes through this. */
  documents: ReadableDocuments;
  /** Predicts + transmits document-mutating operations as one intent (e.g.
   * `WorldSession.dispatchIntent`). */
  dispatchIntent: (ops: WireOperation[]) => void;
  /** Sends one of the eight `combat_*` frames and resolves once its event has applied, or
   * rejects with the server's player-presentable refusal reason. */
  sendCombat: (
    msg: Extract<
      ClientMsg,
      {
        /** Combat frame discriminant literal (matches every `combat_*` tag). */
        type: `combat_${string}`;
      }
    >,
  ) => Promise<void>;
  /** This connection's own user id. */
  selfId: string;
  /** The live world role -- a getter because `Welcome` sets it after construction. */
  role: () => WorldRole | null;
  /** Whether the caller may write `path` on `doc` -- advisory client-side capability check
   * (e.g. `WorldSession.canEdit`). */
  canEdit: (doc: WireDocument, path: string) => boolean;
  /** The world this controller operates in -- `createCombat`/`addCombatants` etc. stamp it on
   * new documents. A getter (not a plain field) because the host constructs this controller
   * before a world is entered (`WorldSession`'s own `world` is `null` until `enter(worldId)`
   * resolves it) -- mirrors `role`'s same construction-before-connection shape. */
  world: () => string;
  /** Structured logger. */
  logger: Logger;
}

/** The combat seam surface: reads, the server-owned clock's intents, and document helpers.
 * See `CombatController`'s own doc for the concrete implementation. */
export interface CombatApi {
  /** Every combat bound to `sceneId`, active first, then by id.
   * @param sceneId The scene to query.
   * @returns Matching combat documents. */
  combatsFor(sceneId: string): WireDocument[];
  /** The scene's single active combat, if any.
   * @param sceneId The scene to query.
   * @returns The active combat document, or `null`. */
  activeFor(sceneId: string): WireDocument | null;
  /** The combat's combatants, in `engine.order` (ids the store cannot resolve are skipped),
   * with any parented combatant absent from `order` appended (id order).
   * @param combatId The combat to query.
   * @returns Ordered combatant documents. */
  combatants(combatId: string): WireDocument[];
  /** The current turn's combatant document.
   * @param combatId The combat to query.
   * @returns The turn's combatant document, or `null` when there is no turn or the store
   * cannot resolve it. */
  turnOf(combatId: string): WireDocument | null;
  /** The latest `"combat"` derived-channel frame; `EMPTY_COMBATS` before the first one
   * arrives. */
  readonly resolved: CombatsView;
  /** One combatant's resolved numbers from the latest `"combat"` frame.
   * @param combatantId The combatant to look up.
   * @returns Its resolved view, or `null` when absent from the latest frame. */
  resolvedFor(combatantId: string): CombatantView | null;
  /** Registers a listener for `resolved`-frame changes (document changes flow through
   * `documents.subscribe` instead).
   * @param listener Called on every new `"combat"` frame.
   * @returns Unsubscribe function. */
  subscribe(listener: () => void): () => void;
  /** Advisory UI-gating affordances for `combatId`.
   * @param combatId The combat to check.
   * @returns The affordance set. */
  canAct(combatId: string): CombatAffordances;
  /** Activates `combatId`.
   * @param combatId The combat to start.
   * @returns Resolves once accepted; rejects with the server's refusal reason. */
  start(combatId: string): Promise<void>;
  /** Deactivates `combatId`.
   * @param combatId The combat to pause.
   * @returns Resolves once accepted; rejects with the server's refusal reason. */
  pause(combatId: string): Promise<void>;
  /** Ends `combatId` (deletes it; children cascade).
   * @param combatId The combat to end.
   * @returns Resolves once accepted; rejects with the server's refusal reason. */
  end(combatId: string): Promise<void>;
  /** Ends the current turn.
   * @param combatId The combat to advance.
   * @returns Resolves once accepted; rejects with the server's refusal reason. */
  advance(combatId: string): Promise<void>;
  /** Steps back one turn record.
   * @param combatId The combat to rewind.
   * @returns Resolves once accepted; rejects with the server's refusal reason. */
  rewind(combatId: string): Promise<void>;
  /** Rebuilds the turn order from current initiatives.
   * @param combatId The combat to sort.
   * @returns Resolves once accepted; rejects with the server's refusal reason. */
  sort(combatId: string): Promise<void>;
  /** Rolls initiative for the named combatants on `channel`.
   * @param combatId The combat.
   * @param channel The chat channel results post to.
   * @param rolls The rolls.
   * @returns Resolves once accepted; rejects with the server's refusal reason. */
  roll(combatId: string, channel: string, rolls: WireCombatRollEntry[]): Promise<void>;
  /** Adjusts one combatant's tracked resource.
   * @param combatId The combat.
   * @param combatantId The target combatant.
   * @param resource The registry key.
   * @param op Delta or set.
   * @returns Resolves once accepted; rejects with the server's refusal reason. */
  modifyResource(combatId: string, combatantId: string, resource: string, op: WireResourceOp): Promise<void>;
  /** Builds and dispatches a new (inactive) combat document at the engine defaults.
   * @param sceneId The scene to bind the combat to.
   * @param opts Optional display name and explicit id.
   * @returns The new combat document's id. */
  createCombat(sceneId: string, opts?: CreateCombatOptions): string;
  /** Dispatches a `Delete` of an inactive combat -- an active one must go through `end()`.
   * @param combatId The combat to delete. */
  deleteCombat(combatId: string): void;
  /** Adds one or more combatants to `combatId` as ONE intent: the `Create`s plus one `/engine/
   * order` append.
   * @param combatId The combat to add to.
   * @param entries The combatants to add.
   * @returns The new combatants' ids, in the same order as `entries`.
   * @throws {CombatClientError} `code: "no-host"` when an entry names neither a resolvable
   * token nor actor. */
  addCombatants(combatId: string, entries: NewCombatant[]): string[];
  /** Adds a one-shot event combatant.
   * @param combatId The combat to add to.
   * @param ev The event to add.
   * @returns The new event combatant's id. */
  addEvent(combatId: string, ev: NewEvent): string;
  /** Removes a combatant: one intent updating `/engine/order` plus a `Delete`.
   * @param combatId The combat.
   * @param combatantId The combatant to remove.
   * @throws {CombatClientError} `code: "turn-owner"` when `combatantId` holds the current
   * turn. */
  removeCombatant(combatId: string, combatantId: string): void;
  /** Hides or reveals a combatant.
   * @param combatantId The combatant to update.
   * @param hidden The new hidden state. */
  setHidden(combatantId: string, hidden: boolean): void;
  /** Reorders a combat's turn order.
   * @param combatId The combat.
   * @param order The new order -- must be the same id set as the current order.
   * @throws {CombatClientError} `code: "order-mismatch"` when the id sets differ. */
  reorder(combatId: string, order: string[]): void;
  /** Sets a combatant's initiative (and optionally tiebreak).
   * @param combatantId The combatant to update.
   * @param initiative The new initiative, or `null` to clear it.
   * @param tiebreak The new tiebreak, when given. */
  setInitiative(combatantId: string, initiative: number | null, tiebreak?: number): void;
}

/**
 * Framework-neutral combat controller: reads compose the optimistic document view with the
 * server-resolved `"combat"` channel; intents dispatch the eight `combat_*` frames; document
 * helpers build/mutate combat documents through ordinary intents. Imports no Svelte and holds
 * no rune -- exposed to Svelte modules as `AppContext.combat: CombatApi` and to every module as
 * the host-provided service `COMBAT_SERVICE`.
 * @example
 * ```ts
 * import { CombatController, type CombatControllerDeps } from "@shadowcat/core";
 *
 * declare const deps: CombatControllerDeps;
 * const combat = new CombatController(deps);
 * combat.combatsFor("scene-1");
 * ```
 */
export class CombatController implements CombatApi {
  /** The latest parsed `"combat"` frame. */
  #resolved: CombatsView = EMPTY_COMBATS;
  /** `resolved`-frame change listeners. */
  #listeners = new Set<() => void>();

  /**
   * @param deps The controller's dependencies (document view, intent dispatch, combat send,
   * identity, capability check).
   * @example
   * ```ts
   * import { CombatController, type CombatControllerDeps } from "@shadowcat/core";
   *
   * declare const deps: CombatControllerDeps;
   * const combat = new CombatController(deps);
   * ```
   */
  constructor(private readonly deps: CombatControllerDeps) {}

  /** Replaces the latest `"combat"` frame and notifies subscribers. Called by the host on
   * every `"combat"` derived-channel frame (never part of `CombatApi` -- a method on the
   * concrete class only).
   * @param view The newly-parsed `"combat"` frame.
   * @example
   * ```ts
   * import { CombatController, EMPTY_COMBATS, type CombatControllerDeps } from "@shadowcat/core";
   *
   * declare const deps: CombatControllerDeps;
   * const combat = new CombatController(deps);
   * combat.setResolved(EMPTY_COMBATS);
   * ```
   */
  setResolved(view: CombatsView): void {
    this.#resolved = view;
    for (const l of this.#listeners) l();
  }

  get resolved(): CombatsView {
    return this.#resolved;
  }

  resolvedFor(combatantId: string): CombatantView | null {
    for (const c of this.#resolved.combats) {
      const found = c.combatants.find((cc) => cc.id === combatantId);
      if (found) return found;
    }
    return null;
  }

  subscribe(listener: () => void): () => void {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  }

  combatsFor(sceneId: string): WireDocument[] {
    return this.deps.documents
      .query("combat")
      .filter((d) => (d.engine as CombatEngine | undefined)?.scene_id === sceneId)
      .sort((a, b) => {
        const ae = (a.engine as CombatEngine).active;
        const be = (b.engine as CombatEngine).active;
        if (ae !== be) return ae ? -1 : 1;
        return a.id < b.id ? -1 : a.id > b.id ? 1 : 0;
      });
  }

  activeFor(sceneId: string): WireDocument | null {
    return this.combatsFor(sceneId).find((d) => (d.engine as CombatEngine).active) ?? null;
  }

  combatants(combatId: string): WireDocument[] {
    const combat = this.deps.documents.get(combatId);
    const engine = combat?.engine as CombatEngine | undefined;
    const order = engine?.order ?? [];
    const seen = new Set<string>();
    const ordered: WireDocument[] = [];
    for (const id of order) {
      const doc = this.deps.documents.get(id);
      if (doc) {
        ordered.push(doc);
        seen.add(id);
      }
    }
    const stray = this.deps.documents
      .query("combatant")
      .filter((d) => d.parent_id === combatId && !seen.has(d.id))
      .sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
    return [...ordered, ...stray];
  }

  turnOf(combatId: string): WireDocument | null {
    const combat = this.deps.documents.get(combatId);
    const turn = (combat?.engine as CombatEngine | undefined)?.turn;
    if (!turn) return null;
    return this.deps.documents.get(turn) ?? null;
  }

  canAct(combatId: string): CombatAffordances {
    const isGm = this.deps.role() === "gm";
    const combat = this.deps.documents.get(combatId);
    const engine = combat?.engine as CombatEngine | undefined;
    const turnDoc = this.turnOf(combatId);
    const isOwnerOf = (doc: WireDocument | null): boolean =>
      !!doc && effectiveOwner(doc, this.deps.documents) === this.deps.selfId;
    const advance = isGm || (engine?.turn_control === "owner_may_end" && isOwnerOf(turnDoc));
    return {
      start: isGm && (engine?.order.length ?? 0) > 0,
      pause: isGm,
      end: isGm,
      advance,
      rewind: isGm && (engine?.round ?? 0) > 0,
      sort: isGm,
      edit: isGm,
      roll: (combatantId) => {
        if (isGm) return true;
        const doc = this.deps.documents.get(combatantId);
        return isOwnerOf(doc ?? null) && !!doc && this.deps.canEdit(doc, "/engine");
      },
      resource: (combatantId) => {
        if (isGm) return true;
        const doc = this.deps.documents.get(combatantId);
        return isOwnerOf(doc ?? null) && !!doc && this.deps.canEdit(doc, "/engine");
      },
    };
  }

  start(combatId: string): Promise<void> {
    return this.deps.sendCombat({ type: "combat_start", request_id: crypto.randomUUID(), combat_id: combatId });
  }

  pause(combatId: string): Promise<void> {
    return this.deps.sendCombat({ type: "combat_pause", request_id: crypto.randomUUID(), combat_id: combatId });
  }

  end(combatId: string): Promise<void> {
    return this.deps.sendCombat({ type: "combat_end", request_id: crypto.randomUUID(), combat_id: combatId });
  }

  advance(combatId: string): Promise<void> {
    return this.deps.sendCombat({ type: "combat_advance", request_id: crypto.randomUUID(), combat_id: combatId });
  }

  rewind(combatId: string): Promise<void> {
    return this.deps.sendCombat({ type: "combat_rewind", request_id: crypto.randomUUID(), combat_id: combatId });
  }

  sort(combatId: string): Promise<void> {
    return this.deps.sendCombat({ type: "combat_sort", request_id: crypto.randomUUID(), combat_id: combatId });
  }

  roll(combatId: string, channel: string, rolls: WireCombatRollEntry[]): Promise<void> {
    return this.deps.sendCombat({
      type: "combat_roll",
      request_id: crypto.randomUUID(),
      combat_id: combatId,
      channel,
      rolls,
    });
  }

  modifyResource(combatId: string, combatantId: string, resource: string, op: WireResourceOp): Promise<void> {
    return this.deps.sendCombat({
      type: "combat_resource",
      request_id: crypto.randomUUID(),
      combat_id: combatId,
      combatant_id: combatantId,
      resource,
      op,
    });
  }

  createCombat(sceneId: string, opts: CreateCombatOptions = {}): string {
    const engine = newCombatEngine(sceneId);
    const doc = buildCombatDoc(this.deps.world(), engine, opts.id);
    doc.name = opts.name ?? null;
    this.deps.dispatchIntent([{ op: "create", doc }]);
    return doc.id;
  }

  deleteCombat(combatId: string): void {
    const doc = this.deps.documents.get(combatId);
    if (!doc) return;
    this.deps.dispatchIntent([{ op: "delete", doc }]);
  }

  addCombatants(combatId: string, entries: NewCombatant[]): string[] {
    const combat = this.deps.documents.get(combatId);
    const engine = combat?.engine as CombatEngine | undefined;
    const oldOrder = engine?.order ?? [];
    const ops: WireOperation[] = [];
    const newIds: string[] = [];
    for (const entry of entries) {
      const token = entry.tokenId ? this.deps.documents.get(entry.tokenId) : undefined;
      const explicitActor = entry.actorId ? this.deps.documents.get(entry.actorId) : undefined;
      const tokenActorId = (
        token?.engine as
          | {
              /** The linked actor's id, when the token carries a link. */
              actor_id?: string | null;
            }
          | undefined
      )?.actor_id;
      const actorId = entry.actorId ?? tokenActorId ?? null;
      if (!entry.tokenId && !actorId) {
        throw new CombatClientError("no-host", "a combatant needs a token or an actor");
      }
      const host = token ?? explicitActor ?? (actorId ? this.deps.documents.get(actorId) : undefined);
      const name = entry.name ?? host?.name ?? null;
      const owner = host ? effectiveOwner(host, this.deps.documents) : null;
      const kind: CombatantKind = { type: "actor", token_id: entry.tokenId ?? null, actor_id: actorId };
      const cengine: CombatantEngine = { kind, initiative: null, tiebreak: 0, resources: {} };
      const doc = buildCombatantDoc(this.deps.world(), combatId, cengine, {
        owner,
        hidden: entry.hidden,
        system: entry.system,
        name,
      });
      ops.push({ op: "create", doc });
      newIds.push(doc.id);
    }
    ops.push({
      op: "update",
      doc_id: combatId,
      changes: [{ path: "/engine/order", old: oldOrder, new: [...oldOrder, ...newIds] }],
    });
    this.deps.dispatchIntent(ops);
    return newIds;
  }

  addEvent(combatId: string, ev: NewEvent): string {
    const combat = this.deps.documents.get(combatId);
    const engine = combat?.engine as CombatEngine | undefined;
    const oldOrder = engine?.order ?? [];
    const kind: CombatantKind = { type: "event", lifespan: ev.lifespan, message: ev.message };
    const cengine: CombatantEngine = { kind, initiative: null, tiebreak: 0, resources: {} };
    const doc = buildCombatantDoc(this.deps.world(), combatId, cengine, {
      hidden: ev.hidden,
      system: ev.system,
      name: ev.name,
    });
    this.deps.dispatchIntent([
      { op: "create", doc },
      {
        op: "update",
        doc_id: combatId,
        changes: [{ path: "/engine/order", old: oldOrder, new: [...oldOrder, doc.id] }],
      },
    ]);
    return doc.id;
  }

  removeCombatant(combatId: string, combatantId: string): void {
    const combat = this.deps.documents.get(combatId);
    const engine = combat?.engine as CombatEngine | undefined;
    if (engine?.turn === combatantId) {
      throw new CombatClientError("turn-owner", "cannot remove the combatant currently taking its turn");
    }
    const oldOrder = engine?.order ?? [];
    const newOrder = oldOrder.filter((id) => id !== combatantId);
    const doc = this.deps.documents.get(combatantId);
    const ops: WireOperation[] = [
      {
        op: "update",
        doc_id: combatId,
        changes: [{ path: "/engine/order", old: oldOrder, new: newOrder }],
      },
    ];
    if (doc) ops.push({ op: "delete", doc });
    this.deps.dispatchIntent(ops);
  }

  setHidden(combatantId: string, hidden: boolean): void {
    const doc = this.deps.documents.get(combatantId);
    if (!doc) return;
    const owner = doc.owner;
    const oldDefault = doc.permissions.default;
    const ops: WireOperation[] = [
      {
        op: "update",
        doc_id: combatantId,
        changes: [{ path: "/permissions/default", old: oldDefault, new: hidden ? "none" : "observer" }],
      },
    ];
    if (owner) {
      const usersPath = "/permissions/users/" + owner;
      const oldEntry = doc.permissions.users[owner];
      if (hidden) {
        ops.push({
          op: "update",
          doc_id: combatantId,
          changes: [{ path: usersPath, old: oldEntry ?? null, remove: true }],
        });
      } else {
        ops.push({
          op: "update",
          doc_id: combatantId,
          changes: [{ path: usersPath, old: oldEntry ?? null, new: "owner" }],
        });
      }
    }
    this.deps.dispatchIntent(ops);
  }

  reorder(combatId: string, order: string[]): void {
    const combat = this.deps.documents.get(combatId);
    const engine = combat?.engine as CombatEngine | undefined;
    const oldOrder = engine?.order ?? [];
    const oldSet = new Set(oldOrder);
    const newSet = new Set(order);
    if (oldSet.size !== newSet.size || [...oldSet].some((id) => !newSet.has(id))) {
      throw new CombatClientError("order-mismatch", "reorder must keep the same combatant id set");
    }
    this.deps.dispatchIntent([
      { op: "update", doc_id: combatId, changes: [{ path: "/engine/order", old: oldOrder, new: order }] },
    ]);
  }

  setInitiative(combatantId: string, initiative: number | null, tiebreak?: number): void {
    const doc = this.deps.documents.get(combatantId);
    const engine = doc?.engine as CombatantEngine | undefined;
    const changes = [{ path: "/engine/initiative", old: engine?.initiative ?? null, new: initiative }];
    if (tiebreak !== undefined) {
      changes.push({ path: "/engine/tiebreak", old: engine?.tiebreak ?? 0, new: tiebreak });
    }
    this.deps.dispatchIntent([{ op: "update", doc_id: combatantId, changes }]);
  }
}
