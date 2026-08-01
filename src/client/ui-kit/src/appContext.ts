import { getContext, setContext } from "svelte";
import type { ContributionRegistry, DocumentStore, ReadableDocuments, AssetResolver, SceneFrame, SceneSubscription, WireOperation, WireDocument, PathResult, MoveStream, WireActorOwnerRef, WireAudience, SheetRef, SubscriptionHandle, WireSearchHit, StampOpts, SyncState } from "@shadowcat/core";
import type { WorldRole } from "@shadowcat/types";
import type { SceneInteraction } from "./sceneInteraction";
import type { ActorSelection } from "./actorSelection.svelte";
import type { TokenSelection } from "./tokenSelection.svelte";
import type { PanelsApi, PanelsChipsView } from "./panelsBridge.svelte";
import type { SceneSelection } from "./sceneSelection.svelte";

/** Translate function shape (framework-neutral; the Svelte adapter supplies a
 * reactive implementation). */
export type TFunc = (key: string, params?: Record<string, string | number>) => string;

/** Chat transport seam (see `AppContext.chat`). Each call resolves when the op
 * is accepted (success-assumed after a short window with no error), and rejects
 * with the server's player-presentable reason on a correlated rejection so the
 * caller can surface it instead of the op silently vanishing. */
export interface ChatApi {
  send(opts: {
    channel: string;
    content: string;
    actorOwner?: WireActorOwnerRef | null;
    audience?: WireAudience;
  }): Promise<void>;
  edit(messageId: string, content: string): Promise<void>;
  delete(messageId: string): Promise<void>;
}

/** Template pull/push/revert/stamp seam (§6.3). Thin orchestration over `store`/`documents` +
 * `dispatchIntent`; the controller opens the conflict modal when needed. */
export interface TemplatesApi {
  /** Deep-clone `source` into a new stamped instance; the caller dispatches the Create. */
  stampInstance(source: WireDocument, opts: StampOpts): WireDocument;
  /** Merge the template into the child; opens the modal on conflicts, else dispatches directly. */
  pull(childId: string): void;
  /** Push the template to every in-store instance the pusher can see + write. */
  push(templateId: string): void;
  /** Reset the child's mergeable bands to the template (keeping placement); refresh base. */
  revert(childId: string): void;
  /** In-store instances stamped from `templateId`. */
  findInstances(templateId: string): WireDocument[];
  /** Provenance/sync state for the sheet badge. */
  syncState(childId: string): SyncState;
  /** Whether the current user may pull/revert this child (owner-or-GM + write caps). */
  canPull(childId: string): boolean;
  /** Whether the current user may push this template (owner-or-GM). */
  canPush(templateId: string): boolean;
}

/**
 * Ambient app state contributed components read via Svelte context. Carries the
 * contribution registry the host renders plus the in-world session essentials
 * (document store, world id, user role). M7d adds the i18n `t`.
 */
export interface AppContext {
  contributions: ContributionRegistry;
  /** Authoritative (confirmed-only) document mirror — the rollback base. */
  store: DocumentStore;
  /** Optimistic (predicted) document view — the canvas render source, so a placed or
   * dragged document shows immediately. */
  documents: ReadableDocuments;
  world: string;
  role: WorldRole;
  /** The caller's SERVER tier, orthogonal to `role` (per-world). Only
   * `"admin"` reaches the server-administration routes, and no world role
   * confers it. Advisory: it decides whether an admin-only control is
   * rendered; the server re-checks every request. */
  serverRole: "admin" | "user";
  /** The current user's id (ownership checks). */
  selfId: string;
  /** Advisory client-side edit gate (mirrors the server's Update-path check) for showing/hiding
   * write controls. The server remains authoritative and re-checks independently at
   * `apply_intent`. **GM ⇒ always true, unconditionally** — the implementation
   * (`worldSession.canEdit`) short-circuits on `role === "gm"` without consulting
   * `doc.permissions.gm_role`, while the server's GM bypass IS conditional on it. So this can
   * over-permit a GM's write affordances on a `gm_role`-capped document; see the caveat on the
   * implementation for the reachability bound. Advisory-only — never treat a `true` here as
   * authorization. */
  canEdit(doc: WireDocument, path: string): boolean;
  /** Open (or focus) a document as a floating sheet panel. `docId` targets a top-level
   * document (optionally one embedded child via `embeddedPath`); `tokenId` resolves to the
   * token's linked actor or embedded actor copy. Fail-closed: a dangling/raw ref opens
   * nothing (logged), never a crash. */
  openDocument(ref: SheetRef): void;
  /** userId → username for the world's members (all roles; used for chat name resolution + GM see-as labels). */
  members: Map<string, string>;
  t: TFunc;
  /** Resolves asset UUIDs to serve URLs, cache-busting on replace. */
  assets: AssetResolver;
  /** Subscribe to asset replace/delete notices; returns an unsubscribe. */
  onAssetChanged(cb: (msg: { uuid: string; op: "replaced" | "deleted" }) => void): () => void;
  /** Subscribe to a SceneDerived channel; the session re-establishes it across
   * reconnects. Returns a synchronous unsubscribe handle. `opts.asUser` (GM-only see-as-player)
   * views the channel as that user; the server rejects it for non-GMs. */
  subscribeScene(
    channel: string,
    onUpdate: (f: SceneFrame) => void,
    opts?: { asUser?: string },
  ): SceneSubscription;
  /** Predict + transmit document operations as one correlated optimistic intent
   * (the module write path). `ctx.client`/`store` reflect the prediction. */
  dispatchIntent(ops: WireOperation[]): void;
  /** Canvas interaction seam: set the active tool, snap to grid, mark a dragged
   * token. No-ops until the Stage attaches the render engine. */
  scene: SceneInteraction;
  /** The actor the place tool stamps; set by module-actors, read by scene-tools. */
  actorSelection: ActorSelection;
  /** Selected token ids for group-select; set by the factions panel, read by the select tool. */
  tokenSelection: TokenSelection;
  /** The scene THIS client renders + subscribes to (M12d). Players follow
   * `world-settings.activeScene`; a GM roaming via `setGmViewedScene` overrides locally. Getter —
   * reactive when read through a `documents.subscribe` bridge. */
  viewedSceneId: string | null;
  /** GM local roam (M12d): view any scene without moving players. No-op for a non-GM. */
  setGmViewedScene: (id: string | null) => void;
  /** Live full-text document search (M6c seam). Resolves once the initial page arrives (and fires
   * `onUpdate` for it); subsequent pushes fire `onUpdate`. Ephemeral — NOT reconnect-resilient;
   * re-subscribe per query. Rejects when there is no transport. */
  searchDocuments: (
    query: string,
    opts: { limit?: number; timeoutMs?: number },
    onUpdate: (hits: WireSearchHit[]) => void,
  ) => Promise<SubscriptionHandle>;
  /** Which scene the game-settings per-scene section edits (M12d "Configure"); set by the scene
   * browser, read by GameSettingsPanel. */
  sceneSelection: SceneSelection;
  /** Broadcast a transient location ping at scene coords on the active scene. */
  sendPing: (x: number, y: number) => void;
  /** Request a grid A* path from `start` through `waypoints` on `scene`. Resolves
   * with the computed path + cost, rejects on unreachable or timeout. Thin
   * transport mirror — no client-side path logic. `token`, when given, names the
   * token the route is for: the server derives the footprint from it and ignores
   * `footprintRadius` entirely. */
  pathfind: (
    scene: string,
    start: [number, number],
    waypoints: [number, number][],
    footprintRadius: number,
    token?: string,
  ) => Promise<PathResult>;
  /** Request server-authoritative move execution for `tokenId` along `path` on
   * `scene`. Resolves with the broadcast `MoveStream` on success; rejects on server
   * rejection or timeout. Animation is broadcast-driven for all viewers via onMoveStream;
   * the resolve value signals success only. */
  moveRequest: (
    scene: string,
    tokenId: string,
    path: [number, number][],
  ) => Promise<MoveStream>;
  /** Subscribe to relayed location pings (incl. our own echo); returns an unsubscribe. */
  onPing: (cb: (msg: { scene: string; x: number; y: number; user: string }) => void) => () => void;
  /** Subscribe to THIS client's own `moveRequest` outcomes (executed/truncated/rejected) — a
   * read-only observability signal, not a broadcast of every scene viewer's moves. Returns an
   * unsubscribe. */
  onMoveOutcome: (cb: (msg: { tokenId: string; outcome: "executed" | "truncated" | "rejected" }) => void) => () => void;
  /** Chat transport seam: send/edit/delete a chat message. NOT fire-and-forget — each frame
   * carries a `request_id` and a rejection correlates back to REJECT this promise with the
   * server's player-presentable reason (which the composer surfaces). Success is the
   * asymmetric case: the broadcast Event echo carries no `request_id`, so an accepted op is
   * never acknowledged and resolution is assumed from silence. See `ChatApi` above. */
  chat: ChatApi;
  /** Template merge seam: stamp + pull/push/revert (M13e). */
  templates: TemplatesApi;
  /** Leave the current world and return to world-select. */
  leaveWorld: () => void;
  /** Log out of the server session and return to the pre-world (login) view. */
  logout: () => Promise<void>;
  /** Narrow per-world UI-state seam. The shell owns storage/persistence; this
   * seam only reads/writes the current world's slice. `getPanelLayout`/
   * `setPanelLayout` persist the panel host's dock/size/order blob (opaque to
   * this seam — the panel host owns its shape). */
  uiState: {
    getPanelLayout(): unknown | null;
    setPanelLayout(blob: unknown): void;
    /** Opaque per-channel chat read-marker blob, owned by the chat module
     * (unread tab badge). */
    getChatRead(): unknown | null;
    setChatRead(blob: unknown): void;
  };
  /** Imperative panel-host seam (open/close/focus/toggle by panel id) plus a
   * live read-only view (`minimized`/`metaMap`/`restore`) for surfaces that
   * render a panel-dock strip elsewhere. No-ops/empty (with a one-time console
   * warning on a write call) until the panel host binds; see `PanelsBridge`. */
  panels: PanelsApi & PanelsChipsView;
}

/** Context key; exported only so test fixtures can seed an AppContext. */
export const __APP_CONTEXT_KEY__ = Symbol("shadowcat.appContext");

/** Publish `ctx` under {@link __APP_CONTEXT_KEY__} for descendants to read via
 * {@link getAppContext}. Call once, from a component that wraps the whole app tree.
 * @param ctx - The AppContext instance to publish.
 * @example setAppContext(buildAppContext(session));
 */
export function setAppContext(ctx: AppContext): void {
  setContext(__APP_CONTEXT_KEY__, ctx);
}

/** Read the {@link AppContext} set by an ancestor via {@link setAppContext}.
 * @returns The published AppContext.
 * @throws {Error} if no ancestor called `setAppContext` — every contribution renders
 * inside a provider, so a thrown error here means the component tree is misassembled,
 * not a normal absence to handle.
 * @example const ctx = getAppContext();
 */
export function getAppContext(): AppContext {
  const ctx = getContext<AppContext | undefined>(__APP_CONTEXT_KEY__);
  if (!ctx) {
    throw new Error("AppContext is not set; render within a provider that calls setAppContext");
  }
  return ctx;
}
