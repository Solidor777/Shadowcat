// Pure unread-tracking logic for the chat panel's tab badge: per-channel
// last-read markers (persisted opaquely via ctx.uiState) and derivation of
// the current unread count / a fresh read-state snapshot from the store's
// messages. No Svelte/store dependency, mirrors channels.ts's shape.
import { parseMessageEngine, type WireDocument } from "@shadowcat/core";

/** A channel's read frontier: the newest message the user has seen there. */
export interface ReadMarker {
  createdAt: number;
  id: string;
}

export type ChatReadState = Record<string, ReadMarker>;

/** True when `doc` is strictly newer than `marker` (or there is no marker at
 * all), using the same created_at-then-id tie-break as `channels.ts`'s
 * `byCreation` — messages share a created_at often enough (bulk seed/import)
 * that id alone is not a safe sole key. */
function isAfter(doc: WireDocument, marker: ReadMarker | undefined): boolean {
  if (!marker) return true;
  if (doc.created_at !== marker.createdAt) return doc.created_at > marker.createdAt;
  return doc.id > marker.id;
}

/** Count of messages newer than each channel's read marker, across every
 * message the store currently holds — NOT scoped to whichever channel view
 * is on screen, since the tab-level badge represents "unread anywhere in
 * chat", independent of the panel's own internal view switch. Excludes the
 * reading user's own posts (never "unread" to their author). */
export function computeUnreadCount(messages: readonly WireDocument[], read: ChatReadState, selfId: string): number {
  let count = 0;
  for (const doc of messages) {
    const sys = parseMessageEngine(doc);
    if (!sys || sys.user_owner === selfId) continue;
    if (isAfter(doc, read[sys.channel])) count++;
  }
  return count;
}

/** Snapshots the newest message per channel across `messages` as a fresh read
 * frontier — called when the tab (re)gains focus, marking everything
 * currently known as seen. */
export function markAllRead(messages: readonly WireDocument[]): ChatReadState {
  const next: ChatReadState = {};
  for (const doc of messages) {
    const sys = parseMessageEngine(doc);
    if (!sys) continue;
    const marker: ReadMarker = { createdAt: doc.created_at, id: doc.id };
    if (isAfter(doc, next[sys.channel])) next[sys.channel] = marker;
  }
  return next;
}
