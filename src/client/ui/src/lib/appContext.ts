import { getContext, setContext } from "svelte";
import type { ContributionRegistry, DocumentStore } from "@shadowcat/core";
import type { WorldRole } from "@shadowcat/types";

/**
 * Ambient app state contributed components read via Svelte context. Carries the
 * contribution registry the host renders plus the in-world session essentials
 * (document store, world id, actor role). M7d adds the i18n `t`.
 */
export interface AppContext {
  contributions: ContributionRegistry;
  store: DocumentStore;
  world: string;
  role: WorldRole;
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
