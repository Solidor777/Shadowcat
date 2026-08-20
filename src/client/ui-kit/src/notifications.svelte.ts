import { createSubscriber } from "svelte/reactivity";
import { NotificationCenter } from "@shadowcat/core";

/** The app's single notification center instance. */
export const notifications = new NotificationCenter();

const subscribe = createSubscriber((update) => notifications.subscribe(update));

/** Reactive read of the active notification list: reading it in a rune context (`$derived`,
 * `$effect`, a component's template) re-runs on any push/dismiss, via the shared `subscribe`.
 * @returns The active notification list, oldest first.
 * @example activeNotifications();
 */
export function activeNotifications() {
  subscribe();
  return notifications.items;
}
