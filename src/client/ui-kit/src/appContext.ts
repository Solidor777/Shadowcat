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
  /** Post a new chat message. Resolves per the class doc's success/rejection contract.
   * @param opts - Message content plus optional attribution/audience.
   * @param opts.channel - The chat channel to post into.
   * @param opts.content - The message body (server-sanitized).
   * @param opts.actorOwner - Optional actor attribution; sent as `null` when omitted.
   * @param opts.audience - Recipient scoping; defaults to `{ kind: "public" }`.
   * @returns Resolves (void) once the send is accepted; rejects with the server's
   * player-presentable reason otherwise. */
  send(opts: {
    /** The chat channel to post into. */
    channel: string;
    /** The message body (server-sanitized). */
    content: string;
    /** Optional actor attribution; sent as `null` when omitted. */
    actorOwner?: WireActorOwnerRef | null;
    /** Recipient scoping; defaults to `{ kind: "public" }`. */
    audience?: WireAudience;
  }): Promise<void>;
  /** Edit an existing message's content. The server enforces edit ownership and
   * rejects via the same correlated-rejection path as `send`.
   * @param messageId - Id of the message to edit.
   * @param content - The replacement body.
   * @returns Resolves once the edit is accepted; rejects on ownership/server refusal. */
  edit(messageId: string, content: string): Promise<void>;
  /** Delete an existing message. The server enforces delete ownership and
   * rejects via the same correlated-rejection path as `send`.
   * @param messageId - Id of the message to delete.
   * @returns Resolves once the delete is accepted; rejects on ownership/server refusal. */
  delete(messageId: string): Promise<void>;
}

/** Template pull/push/revert/stamp seam (§6.3). Thin orchestration over `store`/`documents` +
 * `dispatchIntent`; the controller opens the conflict modal when needed. */
export interface TemplatesApi {
  /** Deep-clone `source` into a new stamped instance; the caller dispatches the Create.
   * @param source - The template document to stamp from.
   * @param opts - Stamping options (placement exclusions, etc.).
   * @returns The new instance document, not yet dispatched. */
  stampInstance(source: WireDocument, opts: StampOpts): WireDocument;
  /** Merge the template into the child; opens the modal on conflicts, else dispatches directly.
   * @param childId - Id of the instance document to pull into. */
  pull(childId: string): void;
  /** Push the template to every in-store instance the pusher can see + write.
   * @param templateId - Id of the template document to push from. */
  push(templateId: string): void;
  /** Reset the child's mergeable bands to the template (keeping placement); refresh base.
   * @param childId - Id of the instance document to revert. */
  revert(childId: string): void;
  /** In-store instances stamped from `templateId`.
   * @param templateId - Id of the template document.
   * @returns The matching in-store instance documents. */
  findInstances(templateId: string): WireDocument[];
  /** Provenance/sync state for the sheet badge.
   * @param childId - Id of the instance document.
   * @returns The child's current sync state relative to its template. */
  syncState(childId: string): SyncState;
  /** Whether the current user may pull/revert this child (owner-or-GM + write caps).
   * @param childId - Id of the instance document.
   * @returns Whether the pull/revert controls should be shown. */
  canPull(childId: string): boolean;
  /** Whether the current user may push this template (owner-or-GM).
   * @param templateId - Id of the template document.
   * @returns Whether the push control should be shown. */
  canPush(templateId: string): boolean;
}

/**
 * Ambient app state contributed components read via Svelte context. Carries the
 * contribution registry the host renders plus the in-world session essentials
 * (document store, world id, user role, and the i18n `t` function).
 */
export interface AppContext {
  /** The registry the panel/surface host renders contributed UI from. */
  contributions: ContributionRegistry;
  /** Authoritative (confirmed-only) document mirror — the rollback base. */
  store: DocumentStore;
  /** Optimistic (predicted) document view — the canvas render source, so a placed or
   * dragged document shows immediately. */
  documents: ReadableDocuments;
  /** The current world's id. */
  world: string;
  /** The current user's world-scoped role. */
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
   * authorization.
   * @param doc - The document the caller wants to write to.
   * @param path - The field path within `doc` being edited.
   * @returns Whether write controls for that path should render as enabled. */
  canEdit(doc: WireDocument, path: string): boolean;
  /** Open (or focus) a document as a floating sheet panel. `docId` targets a top-level
   * document (optionally one embedded child via `embeddedPath`); `tokenId` resolves to the
   * token's linked actor or embedded actor copy. Fail-closed: a dangling/raw ref opens
   * nothing (logged), never a crash.
   * @param ref - The document/token/embedded-child reference to open. */
  openDocument(ref: SheetRef): void;
  /** userId → username for the world's members (all roles; used for chat name resolution + GM see-as labels). */
  members: Map<string, string>;
  /** Translate a key to the active locale's string. */
  t: TFunc;
  /** Resolves asset UUIDs to serve URLs, cache-busting on replace. */
  assets: AssetResolver;
  /** Subscribe to asset replace/delete notices; returns an unsubscribe.
   * @param cb - Called with the changed asset's uuid and the operation kind.
   * @returns Unsubscribe function. */
  onAssetChanged(cb: (msg: {
    /** Id of the asset that changed. */
    uuid: string;
    /** Whether the asset was replaced (new content, same id) or deleted. */
    op: "replaced" | "deleted";
  }) => void): () => void;
  /** Subscribe to a SceneDerived channel; the session re-establishes it across
   * reconnects. Returns a synchronous unsubscribe handle. `opts.asUser` (GM-only see-as-player)
   * views the channel as that user; the server rejects it for non-GMs.
   * @param channel - The SceneDerived channel name.
   * @param onUpdate - Called with each pushed frame.
   * @param opts - Optional GM see-as-player override.
   * @param opts.asUser - Userid to view the channel as; GM-only, server-rejected otherwise.
   * @returns A subscription handle carrying a synchronous unsubscribe. */
  subscribeScene(
    channel: string,
    onUpdate: (f: SceneFrame) => void,
    opts?: {
      /** Userid to view the channel as; GM-only, server-rejected otherwise. */
      asUser?: string;
    },
  ): SceneSubscription;
  /** Predict + transmit document operations as one correlated optimistic intent
   * (the module write path). `ctx.client`/`store` reflect the prediction.
   * @param ops - The wire operations to apply as one intent. */
  dispatchIntent(ops: WireOperation[]): void;
  /** Canvas interaction seam: set the active tool, snap to grid, mark a dragged
   * token. No-ops until the Stage attaches the render engine. */
  scene: SceneInteraction;
  /** The actor the place tool stamps; set by module-actors, read by scene-tools. */
  actorSelection: ActorSelection;
  /** Selected token ids for group-select; set by the factions panel, read by the select tool. */
  tokenSelection: TokenSelection;
  /** The scene THIS client renders + subscribes to. Players follow
   * `world-settings.activeScene`; a GM roaming via `setGmViewedScene` overrides locally. Getter —
   * reactive when read through a `documents.subscribe` bridge. */
  viewedSceneId: string | null;
  /** GM local roam: view any scene without moving players. No-op for a non-GM. */
  setGmViewedScene: (id: string | null) => void;
  /** Live full-text document search seam. Resolves once the initial page arrives (and fires
   * `onUpdate` for it); subsequent pushes fire `onUpdate`. Ephemeral — NOT reconnect-resilient;
   * re-subscribe per query. Rejects when there is no transport. */
  searchDocuments: (
    query: string,
    opts: {
      /** Max hits per page; server-defaulted when omitted. */
      limit?: number;
      /** Milliseconds to wait for the initial page before rejecting; server-defaulted when omitted. */
      timeoutMs?: number;
    },
    onUpdate: (hits: WireSearchHit[]) => void,
  ) => Promise<SubscriptionHandle>;
  /** Which scene the game-settings per-scene section edits ("Configure"); set by the scene
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
  onPing: (cb: (msg: {
    /** Scene the ping was placed on. */
    scene: string;
    /** Scene-space x coordinate. */
    x: number;
    /** Scene-space y coordinate. */
    y: number;
    /** Userid of the pinging user. */
    user: string;
  }) => void) => () => void;
  /** Subscribe to THIS client's own `moveRequest` outcomes (executed/truncated/rejected) — a
   * read-only observability signal, not a broadcast of every scene viewer's moves. Returns an
   * unsubscribe. */
  onMoveOutcome: (cb: (msg: {
    /** Token the move was requested for. */
    tokenId: string;
    /** `truncated` means the server stopped the move short (e.g. a movement-gate rejection
     * mid-path); `rejected` means no move happened at all. */
    outcome: "executed" | "truncated" | "rejected";
  }) => void) => () => void;
  /** Chat transport seam: send/edit/delete a chat message. NOT fire-and-forget — each frame
   * carries a `request_id` and a rejection correlates back to REJECT this promise with the
   * server's player-presentable reason (which the composer surfaces). Success is the
   * asymmetric case: the broadcast Event echo carries no `request_id`, so an accepted op is
   * never acknowledged and resolution is assumed from silence. See `ChatApi` above. */
  chat: ChatApi;
  /** Template merge seam: stamp + pull/push/revert. */
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
    /** Read the persisted panel-layout blob.
     * @returns The persisted panel-layout blob, or `null` if never saved. */
    getPanelLayout(): unknown | null;
    /** Persist the panel-layout blob.
     * @param blob - The panel host's dock/size/order blob to persist. */
    setPanelLayout(blob: unknown): void;
    /** Opaque per-channel chat read-marker blob, owned by the chat module
     * (unread tab badge).
     * @returns The persisted read-marker blob, or `null` if never saved. */
    getChatRead(): unknown | null;
    /** Persist the chat read-marker blob.
     * @param blob - The chat module's read-marker blob to persist. */
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
