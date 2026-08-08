import type { AppContext } from "../appContext";
import { __APP_CONTEXT_KEY__ } from "../appContext";
import { DocumentStore, AssetResolver, ContributionRegistry, silentLogger } from "@shadowcat/core";
import { SceneInteractionBridge } from "../sceneInteraction";
import { ActorSelection } from "../actorSelection.svelte";
import { TokenSelection } from "../tokenSelection.svelte";
import { PanelsBridge } from "../panelsBridge.svelte";
import { SceneSelection } from "../sceneSelection.svelte";

/**
 * Build a Map for @testing-library/svelte's `context` option holding a minimal
 * AppContext (overridable per field), seeded under the real private key.
 *
 * Fidelity gap: in production, `documents` (the optimistic view) and `store` (the authoritative
 * `DocumentStore`) are INDEPENDENT siblings — `OptimisticClient` is constructed from
 * `(selfId, logger)` and never receives `store`; each is fed the same `applyCommand` separately
 * (see `WorldSession`). Here, `documents` defaults to `over.documents ?? over.store
 * ?? new DocumentStore()`: if a test overrides only `store`, `documents` is that SAME
 * plain `DocumentStore` instance, not an `OptimisticClient` over it. Optimistic-specific
 * behavior (predicted-op overlay, rollback on reject) is therefore NOT emulated unless
 * the test supplies its own `documents` override — reads through `documents` in the
 * default case are plain authoritative-store reads.
 * @param over - Per-field overrides; every field not given falls back to a
 * harmless no-op/empty default (see the field list in the body).
 * @returns A context Map suitable for @testing-library/svelte's `context` render option.
 * @example
 * render(MyPanel, { context: setAppContextForTest({ role: "gm" }) });
 */
export function setAppContextForTest(over: Partial<AppContext> = {}): Map<unknown, unknown> {
  const ctx: AppContext = {
    contributions: over.contributions ?? new ContributionRegistry(),
    store: over.store ?? new DocumentStore(),
    documents: over.documents ?? over.store ?? new DocumentStore(),
    assets: over.assets ?? new AssetResolver(),
    world: over.world ?? "w1",
    role: over.role ?? "gm",
    // Defaults to a plain user: a world GM is not a server admin, so
    // admin-only surfaces stay hidden unless a test opts in explicitly.
    serverRole: over.serverRole ?? "user",
    selfId: over.selfId ?? "u-self",
    canEdit: over.canEdit ?? (() => true),
    openDocument: over.openDocument ?? (() => {}),
    members: over.members ?? new Map(),
    t: over.t ?? ((k: string) => k),
    onAssetChanged: over.onAssetChanged ?? (() => () => {}),
    subscribeScene: over.subscribeScene ?? (() => ({ unsubscribe() {} })),
    dispatchIntent: over.dispatchIntent ?? (() => {}),
    scene: over.scene ?? new SceneInteractionBridge(),
    actorSelection: over.actorSelection ?? new ActorSelection(),
    tokenSelection: over.tokenSelection ?? new TokenSelection(),
    sendPing: over.sendPing ?? (() => {}),
    pathfind: over.pathfind ?? (() => Promise.reject(new Error("not connected"))),
    moveRequest: over.moveRequest ?? (() => Promise.reject(new Error("not connected"))),
    onPing: over.onPing ?? (() => () => {}),
    onMoveOutcome: over.onMoveOutcome ?? (() => () => {}),
    chat: over.chat ?? { send: () => Promise.resolve(), edit: () => Promise.resolve(), delete: () => Promise.resolve() },
    leaveWorld: over.leaveWorld ?? (() => {}),
    logout: over.logout ?? (async () => {}),
    uiState: over.uiState ?? {
      getPanelLayout: () => null,
      setPanelLayout: () => {},
      getChatRead: () => null,
      setChatRead: () => {},
    },
    panels: over.panels ?? new PanelsBridge(silentLogger),
    viewedSceneId: over.viewedSceneId ?? null,
    setGmViewedScene: over.setGmViewedScene ?? (() => {}),
    searchDocuments: over.searchDocuments ?? (() => Promise.reject(new Error("not connected"))),
    sceneSelection: over.sceneSelection ?? new SceneSelection(),
    templates: over.templates ?? {
      stampInstance: (s) => s,
      pull: () => {},
      push: () => {},
      revert: () => {},
      findInstances: () => [],
      syncState: () => "none",
      canPull: () => false,
      canPush: () => false,
    },
  };
  return new Map([[__APP_CONTEXT_KEY__, ctx]]);
}
