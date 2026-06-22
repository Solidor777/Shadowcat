import { getContext, setContext } from "svelte";
import type { ContributionRegistry, DocumentStore, ReadableDocuments, AssetResolver, SceneFrame, SceneSubscription, WireOperation } from "@shadowcat/core";
import type { WorldRole } from "@shadowcat/types";
import type { SceneInteraction } from "./sceneInteraction";

/**
 * Ambient app state contributed components read via Svelte context. Carries the
 * contribution registry the host renders plus the in-world session essentials
 * (document store, world id, actor role). M7d adds the i18n `t`.
 */
/** Translate function shape (framework-neutral; the Svelte adapter supplies a
 * reactive implementation). */
export type TFunc = (key: string, params?: Record<string, string | number>) => string;

export interface AppContext {
  contributions: ContributionRegistry;
  /** Authoritative (confirmed-only) document mirror — the rollback base. */
  store: DocumentStore;
  /** Optimistic (predicted) document view — the canvas render source, so a placed or
   * dragged document shows immediately. */
  documents: ReadableDocuments;
  world: string;
  role: WorldRole;
  t: TFunc;
  /** Resolves asset UUIDs to serve URLs, cache-busting on replace. */
  assets: AssetResolver;
  /** Subscribe to asset replace/delete notices; returns an unsubscribe. */
  onAssetChanged(cb: (msg: { uuid: string; op: "replaced" | "deleted" }) => void): () => void;
  /** Subscribe to a SceneDerived channel; the session re-establishes it across
   * reconnects. Returns a synchronous unsubscribe handle. */
  subscribeScene(channel: string, onUpdate: (f: SceneFrame) => void): SceneSubscription;
  /** Predict + transmit document operations as one correlated optimistic intent
   * (the module write path). `ctx.client`/`store` reflect the prediction. */
  dispatchIntent(ops: WireOperation[]): void;
  /** Canvas interaction seam: set the active tool, snap to grid, mark a dragged
   * token. No-ops until the Stage attaches the render engine. */
  scene: SceneInteraction;
  /** Broadcast a transient location ping at scene coords on the active scene. */
  sendPing: (x: number, y: number) => void;
  /** Subscribe to relayed location pings (incl. our own echo); returns an unsubscribe. */
  onPing: (cb: (msg: { scene: string; x: number; y: number; user: string }) => void) => () => void;
  /** Leave the current world and return to world-select. */
  leaveWorld: () => void;
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
