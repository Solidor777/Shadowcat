import type { PanelBadge } from "@shadowcat/core";
import type { CombatTurnEvent } from "@shadowcat/core";

/** The tracker's panel-tab badge: 1 while it is the bound viewer's own turn, 0 otherwise.
 * Unbound (before `CombatTrackerPanel` binds it on mount), it stays at 0 and never notifies —
 * a keep-mounted panel's badge must not fabricate a count before it knows what "mine" means. */
export class TurnBadge implements PanelBadge {
  /** Current badge count. */
  #count = 0;
  /** Resolves whether a combatant id is the bound viewer's own, once bound. */
  #isMine: ((combatantId: string) => boolean) | null = null;
  /** Called once per own-turn start, once bound. */
  #notify: (() => void) | null = null;
  /** `get()`/`subscribe()` listeners. */
  #listeners = new Set<() => void>();

  /** The current badge count.
   * @returns 1 on the bound viewer's own turn, 0 otherwise.
   * @example
   * ```
   * const badge = new TurnBadge();
   * badge.get(); // 0
   * ```
   */
  get(): number {
    return this.#count;
  }

  /** Registers a listener called after every count change.
   * @param cb Called with no arguments on every change.
   * @returns An unsubscribe function.
   * @example
   * ```
   * const badge = new TurnBadge();
   * const unsubscribe = badge.subscribe(() => console.log(badge.get()));
   * ```
   */
  subscribe(cb: () => void): () => void {
    this.#listeners.add(cb);
    return () => this.#listeners.delete(cb);
  }

  /** Binds the badge to a viewer identity and a one-shot own-turn notifier. Idempotent to call
   * more than once (a keep-mounted panel binds once per mount).
   * @param isMine Resolves whether a combatant id belongs to the bound viewer.
   * @param notify Called once per own-turn start.
   * @example
   * ```
   * const badge = new TurnBadge();
   * declare const selfCombatantId: string;
   * badge.bind((id) => id === selfCombatantId, () => {});
   * ```
   */
  bind(isMine: (combatantId: string) => boolean, notify: () => void): void {
    this.#isMine = isMine;
    this.#notify = notify;
  }

  /** Handles `combat:turn-start`: counts 1 and notifies when bound and the started combatant is
   * the viewer's own; 0 otherwise.
   * @param p The turn-start payload.
   * @example
   * ```
   * const badge = new TurnBadge();
   * badge.onTurnStart({ combatId: "c1", combatantId: "cc1", round: 1, kind: "actor" });
   * ```
   */
  onTurnStart(p: CombatTurnEvent): void {
    if (this.#isMine && this.#isMine(p.combatantId)) {
      this.#set(1);
      this.#notify?.();
    } else {
      this.#set(0);
    }
  }

  /** Handles `combat:turn-end`: resets to 0 when it names the combatant currently holding the
   * badge's count (the turn that just ended).
   * @param _p The turn-end payload (unused — any turn ending clears the badge).
   * @example
   * ```
   * const badge = new TurnBadge();
   * badge.onTurnEnd({ combatId: "c1", combatantId: "cc1", round: 1, kind: "actor" });
   * ```
   */
  onTurnEnd(_p: CombatTurnEvent): void {
    this.#set(0);
  }

  /** Handles `combat:end`: resets to 0.
   * @example
   * ```
   * const badge = new TurnBadge();
   * badge.clear();
   * ```
   */
  clear(): void {
    this.#set(0);
  }

  /** Sets the count and notifies listeners only when it actually changed.
   * @param count The new count.
   * @example
   * ```
   * // private method; not part of the public API — invoked from every handler above
   * this.#set(1);
   * ```
   */
  #set(count: number): void {
    if (this.#count === count) return;
    this.#count = count;
    for (const l of this.#listeners) l();
  }
}
