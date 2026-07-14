import { getContext, setContext } from "svelte";
import type { ContributionRegistry, DocumentStore, ReadableDocuments, AssetResolver, SceneFrame, SceneSubscription, WireOperation, WireDocument, PathResult, MoveStream, WireActorOwnerRef, WireAudience } from "@shadowcat/core";
import type { WorldRole } from "@shadowcat/types";
import type { SceneInteraction } from "./sceneInteraction";
import type { ActorSelection } from "./actorSelection.svelte";
import type { TokenSelection } from "./tokenSelection.svelte";
import type { PanelsApi, PanelsChipsView } from "./panelsBridge.svelte";

/**
 * Ambient app state contributed components read via Svelte context. Carries the
 * contribution registry the host renders plus the in-world session essentials
 * (document store, world id, user role). M7d adds the i18n `t`.
 */
/** Translate function shape (framework-neutral; the Svelte adapter supplies a
 * reactive implementation). */
export type TFunc = (key: string, params?: Record<string, string | number>) => string;

/** Chat transport seam (see `AppContext.chat`). Fire-and-forget: none of these
 * calls resolve/reject — the server applies or rejects out-of-band. */
export interface ChatApi {
  send(opts: {
    channel: string;
    content: string;
    actorOwner?: WireActorOwnerRef | null;
    audience?: WireAudience;
  }): void;
  edit(messageId: string, content: string): void;
  delete(messageId: string): void;
}

export interface AppContext {
  contributions: ContributionRegistry;
  /** Authoritative (confirmed-only) document mirror — the rollback base. */
  store: DocumentStore;
  /** Optimistic (predicted) document view — the canvas render source, so a placed or
   * dragged document shows immediately. */
  documents: ReadableDocuments;
  world: string;
  role: WorldRole;
  /** The current user's id (ownership checks). */
  selfId: string;
  /** Advisory client-side edit gate (mirrors the server's Update-path check) for showing/hiding
   * write controls. The server remains authoritative. GM ⇒ always true. */
  canEdit(doc: WireDocument, path: string): boolean;
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
  /** Broadcast a transient location ping at scene coords on the active scene. */
  sendPing: (x: number, y: number) => void;
  /** Request a grid A* path from `start` through `waypoints` on `scene`. Resolves
   * with the computed path + cost, rejects on unreachable or timeout. Thin
   * transport mirror — no client-side path logic. */
  pathfind: (
    scene: string,
    start: [number, number],
    waypoints: [number, number][],
    footprintRadius: number,
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
  /** Chat transport seam: send/edit/delete a chat message. Fire-and-forget (no
   * correlation id) — the server applies/rejects out-of-band; the composer
   * pre-validates the cheap rejects client-side. */
  chat: ChatApi;
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
  };
  /** Imperative panel-host seam (open/close/focus/toggle by panel id) plus a
   * live read-only view (`minimized`/`metaMap`/`restore`) for surfaces that
   * render a panel-dock strip elsewhere. No-ops/empty (with a one-time console
   * warning on a write call) until the panel host binds; see `PanelsBridge`. */
  panels: PanelsApi & PanelsChipsView;
}

/** Context key; exported only so test fixtures can seed an AppContext. */
export const __APP_CONTEXT_KEY__ = Symbol("shadowcat.appContext");

export function setAppContext(ctx: AppContext): void {
  setContext(__APP_CONTEXT_KEY__, ctx);
}

export function getAppContext(): AppContext {
  const ctx = getContext<AppContext | undefined>(__APP_CONTEXT_KEY__);
  if (!ctx) {
    throw new Error("AppContext is not set; render within a provider that calls setAppContext");
  }
  return ctx;
}
