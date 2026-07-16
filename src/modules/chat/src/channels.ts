// Pure chat view model: which channel/GM view is active, filtering, sort, post
// target derivation. No Svelte/store dependency — ChatPanel wires this to reactive
// queries. `channel` itself is a purely client-side label (chat skill: the server
// enforces only `audience`, never `channel`); "All" and per-channel views both read
// EVERY message regardless of audience, while the GM view filters on `audience`.
import type { ChatMessageEngine, WireAudience, WireDocument } from "@shadowcat/core";

export type ChatView = { kind: "all" } | { kind: "channel"; id: string } | { kind: "gm" };

/** Post target for a view: All → the default channel; GM → gm_only audience. */
export function postTarget(view: ChatView): { channel: string; audience: WireAudience } {
  if (view.kind === "channel") return { channel: view.id, audience: { kind: "public" } };
  if (view.kind === "gm") return { channel: "general", audience: { kind: "gm_only" } };
  return { channel: "general", audience: { kind: "public" } };
}

export function inView(view: ChatView, sys: ChatMessageEngine): boolean {
  if (view.kind === "all") return true;
  if (view.kind === "gm") return sys.audience.kind === "gm_only";
  return sys.channel === view.id;
}

/** Sort by envelope created_at then id (server-set; stable under edits). */
export function byCreation(a: WireDocument, b: WireDocument): number {
  return a.created_at - b.created_at || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
}

export const RENDER_CAP = 200;
