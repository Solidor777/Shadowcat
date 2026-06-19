import { getContext, setContext } from "svelte";
import type { ContributionRegistry } from "@shadowcat/core";

/**
 * Ambient app state contributed components read via Svelte context. M7b-3 carries
 * the contribution registry the host renders; M7c adds store/world/role (shell +
 * Welcome) and M7d adds the i18n `t`. Extend this interface there.
 */
export interface AppContext {
  contributions: ContributionRegistry;
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
