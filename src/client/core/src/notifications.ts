// Framework-neutral UI-visible notification primitive, mirroring I18n's shape (subscribe/snapshot
// reactivity so any framework can read it reactively) — distinct from Logger, which stays a
// developer-facing diagnostic sink writing only to the console. This is the user-facing channel
// for operation-level feedback ("an operation partially applied", "some targets were skipped").
/** A notification's severity, driving its visual treatment in the host UI. */
export type NotificationLevel = "info" | "warning" | "error";

/** An in-flight operation's progress, rendered as a bar beneath the message. */
export interface NotificationProgress {
  /** Current progress toward `max`; `null` marks an indeterminate operation (no numeric claim). */
  value: number | null;
  /** The determinate total `value` counts toward; ignored when `value` is `null`. */
  max: number;
}

/** One active notification. */
export interface Notification {
  /** Unique id, assigned at push time — used to target `dismiss`. */
  id: string;
  /** Severity, driving visual treatment. */
  level: NotificationLevel;
  /** The message text, already resolved/interpolated — this channel does not itself do i18n
   * lookup; a caller passes the final string (call `t(key, params)` before `push` if needed). */
  message: string;
  /** An optional call-to-action button: `label` is pre-resolved like
   * `message` (this channel does no i18n lookup), and `run` is the callback a
   * click invokes — e.g. an affordance needing a fresh user gesture (a
   * browser-blocked window reopen) carries its gesture here. */
  action?: {
    /** The button's label text, already resolved/interpolated. */
    label: string;
    /** Invoked once when the user clicks the action; the notification
     * dismisses itself alongside. */
    run: () => void;
  };
  /** Present while the notification tracks an in-flight operation; the host never auto-dismisses
   * it and never dismisses it on a row click, so the operation's owner ends it with `dismiss`. */
  progress?: NotificationProgress;
}

/** A `subscribe()` callback, invoked with no arguments on any push/dismiss. */
export type NotificationListener = () => void;

/** The most `Notification`s `NotificationCenter.items` ever exposes at once — a host rendering
 * unboundedly many toasts degrades to an unreadable stack. Excess pushes queue FIFO and surface
 * as each visible notification dismisses (auto or manual), never dropped. */
export const MAX_VISIBLE_NOTIFICATIONS = 5;

/** Framework-neutral notification center: an ordered list of active notifications plus
 * `subscribe`/snapshot reactivity (mirrors `I18n`/`ContributionRegistry`) so any framework
 * (Svelte via `createSubscriber`, Vue, …) can read `items` reactively. Caps the VISIBLE set at
 * `MAX_VISIBLE_NOTIFICATIONS`; a push beyond the cap queues FIFO and is promoted to visible as
 * a visible notification dismisses — `items` never exposes a queued entry, so a consumer keying
 * behaviour off "is this notification visible" (e.g. auto-dismiss timing) needs no separate
 * check. */
export class NotificationCenter {
  /** Visible notifications, oldest first — exactly what `items` exposes. */
  #items: Notification[] = [];
  /** Notifications pushed beyond the visible cap, oldest first; promoted into `#items` FIFO as
   * a visible slot frees up. */
  #queue: Notification[] = [];
  /** Counter backing the next `push`-assigned id. */
  #nextId = 0;
  /** Subscribers notified on every push/dismiss. */
  #listeners = new Set<NotificationListener>();

  /** Every currently VISIBLE notification, oldest first — never includes a queued (over-cap)
   * notification.
   * @returns The visible notification list. */
  get items(): readonly Notification[] {
    return this.#items;
  }

  /** Adds a notification and notifies subscribers.
   * @param level Severity, driving visual treatment.
   * @param message The message text (already resolved/interpolated).
   * @param action An optional call-to-action (`{ label, run }`, both
   * pre-resolved) the host renders as a button beside the dismiss control.
   * @param progress An optional in-flight progress the host renders as a bar; update it with `update`.
   * @returns The new notification's id, usable with `dismiss` and `update`.
   * @example
   * ```ts
   * import { NotificationCenter } from "@shadowcat/core";
   *
   * const center = new NotificationCenter();
   * const id = center.push("warning", "Some targets were skipped.");
   * center.push("info", "Windows can be restored.", { label: "Reopen", run: () => {} });
   * center.push("info", "Uploading", undefined, { value: 0, max: 100 });
   * center.dismiss(id);
   * ```
   */
  push(
    level: NotificationLevel,
    message: string,
    action?: Notification["action"],
    progress?: NotificationProgress,
  ): string {
    const id = `n${this.#nextId++}`;
    const notification: Notification = {
      id,
      level,
      message,
      ...(action ? { action } : {}),
      ...(progress ? { progress } : {}),
    };
    if (this.#items.length < MAX_VISIBLE_NOTIFICATIONS) {
      // A NEW array, never an in-place push: `items` is read through a reference-equality
      // reactive bridge (`activeNotifications()` in a `$derived`), which observes a changed
      // reference and nothing else — `dismiss` replaces the array for the same reason.
      this.#items = [...this.#items, notification];
    } else {
      this.#queue = [...this.#queue, notification];
    }
    this.#emit();
    return id;
  }

  /** Replaces a notification's message and/or progress in place, whether it is visible or still
   * queued; a no-op (no emit) if `id` matches neither. The id and queue position are unchanged,
   * and the containing array is replaced so a reference-equality reactive read observes it.
   * @param id The notification id, as returned by `push`.
   * @param patch The fields to replace; absent fields are kept.
   * @example
   * ```ts
   * import { NotificationCenter } from "@shadowcat/core";
   *
   * const center = new NotificationCenter();
   * const id = center.push("info", "Uploading", undefined, { value: 0, max: 100 });
   * center.update(id, { progress: { value: 50, max: 100 } });
   * ```
   */
  update(id: string, patch: Partial<Pick<Notification, "message" | "progress">>): void {
    if (this.#items.some((n) => n.id === id)) {
      this.#items = this.#items.map((n) => (n.id === id ? { ...n, ...patch } : n));
    } else if (this.#queue.some((n) => n.id === id)) {
      this.#queue = this.#queue.map((n) => (n.id === id ? { ...n, ...patch } : n));
    } else {
      return;
    }
    this.#emit();
  }

  /** Removes a notification by id, whether currently visible or still queued; a no-op if `id`
   * matches neither. Dismissing a VISIBLE notification promotes the oldest queued one (if any)
   * into the freed slot in the same update.
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
    const beforeVisible = this.#items.length;
    this.#items = this.#items.filter((n) => n.id !== id);
    if (this.#items.length !== beforeVisible) {
      // A visible slot freed up: promote the oldest queued notification, if any.
      if (this.#queue.length > 0) {
        const [next, ...rest] = this.#queue;
        this.#items = [...this.#items, next];
        this.#queue = rest;
      }
      this.#emit();
      return;
    }
    const beforeQueued = this.#queue.length;
    this.#queue = this.#queue.filter((n) => n.id !== id);
    if (this.#queue.length !== beforeQueued) this.#emit();
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
