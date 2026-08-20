// Framework-neutral UI-visible notification primitive, mirroring I18n's shape (subscribe/snapshot
// reactivity so any framework can read it reactively) — distinct from Logger, which stays a
// developer-facing diagnostic sink writing only to the console. This is the user-facing channel
// for operation-level feedback ("an operation partially applied", "some targets were skipped").
/** A notification's severity, driving its visual treatment in the host UI. */
export type NotificationLevel = "info" | "warning" | "error";

/** One active notification. */
export interface Notification {
  /** Unique id, assigned at push time — used to target `dismiss`. */
  id: string;
  /** Severity, driving visual treatment. */
  level: NotificationLevel;
  /** The message text, already resolved/interpolated — this channel does not itself do i18n
   * lookup; a caller passes the final string (call `t(key, params)` before `push` if needed). */
  message: string;
}

/** A `subscribe()` callback, invoked with no arguments on any push/dismiss. */
export type NotificationListener = () => void;

/** Framework-neutral notification center: an ordered list of active notifications plus
 * `subscribe`/snapshot reactivity (mirrors `I18n`/`ContributionRegistry`) so any framework
 * (Svelte via `createSubscriber`, Vue, …) can read `items` reactively. */
export class NotificationCenter {
  /** Active notifications, oldest first. */
  #items: Notification[] = [];
  /** Counter backing the next `push`-assigned id. */
  #nextId = 0;
  /** Subscribers notified on every push/dismiss. */
  #listeners = new Set<NotificationListener>();

  /** Every currently active notification, oldest first.
   * @returns The active notification list. */
  get items(): readonly Notification[] {
    return this.#items;
  }

  /** Adds a notification and notifies subscribers.
   * @param level Severity, driving visual treatment.
   * @param message The message text (already resolved/interpolated).
   * @returns The new notification's id, usable with `dismiss`.
   * @example
   * ```ts
   * import { NotificationCenter } from "@shadowcat/core";
   *
   * const center = new NotificationCenter();
   * const id = center.push("warning", "Some targets were skipped.");
   * center.dismiss(id);
   * ```
   */
  push(level: NotificationLevel, message: string): string {
    const id = `n${this.#nextId++}`;
    this.#items.push({ id, level, message });
    this.#emit();
    return id;
  }

  /** Removes a notification by id; a no-op if `id` is not currently active.
   * @param id The notification id, as returned by `push`.
   * @example
   * ```ts
   * import { NotificationCenter } from "@shadowcat/core";
   *
   * const center = new NotificationCenter();
   * const id = center.push("info", "Saved.");
   * center.dismiss(id);
   * ```
   */
  dismiss(id: string): void {
    const before = this.#items.length;
    this.#items = this.#items.filter((n) => n.id !== id);
    if (this.#items.length !== before) this.#emit();
  }

  /** Notifies `listener` on every push/dismiss.
   * @param listener Called with no arguments after a change.
   * @returns An unsubscribe function.
   * @example
   * ```ts
   * import { NotificationCenter } from "@shadowcat/core";
   *
   * const center = new NotificationCenter();
   * const unsubscribe = center.subscribe(() => {});
   * unsubscribe();
   * ```
   */
  subscribe(listener: NotificationListener): () => void {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  }

  /** Notifies every subscriber.
   * @example
   * ```
   * // private method; not part of the public API — called by push/dismiss
   * this.#emit();
   * ```
   */
  #emit(): void {
    for (const fn of this.#listeners) fn();
  }
}
