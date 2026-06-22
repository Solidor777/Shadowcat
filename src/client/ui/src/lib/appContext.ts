import { getContext, setContext } from "svelte";
import type { ContributionRegistry, DocumentStore, AssetResolver } from "@shadowcat/core";
import type { WorldRole } from "@shadowcat/types";

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
  store: DocumentStore;
  world: string;
  role: WorldRole;
  t: TFunc;
  /** Resolves asset UUIDs to serve URLs, cache-busting on replace. */
  assets: AssetResolver;
  /** Subscribe to asset replace/delete notices; returns an unsubscribe. */
  onAssetChanged(cb: (msg: { uuid: string; op: "replaced" | "deleted" }) => void): () => void;
  /** Leave the current world and return to world-select. */
  leaveWorld: () => void;
}

const KEY = Symbol("shadowcat.appContext");

export function setAppContext(ctx: AppContext): void {
  setContext(KEY, ctx);
}

export function getAppContext(): AppContext {
  const ctx = getContext<AppContext | undefined>(KEY);
  if (!ctx) {
    throw new Error("AppContext is not set; render within a provider that calls setAppContext");
  }
  return ctx;
}
