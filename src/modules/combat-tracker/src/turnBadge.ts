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

  get(): number {
    return this.#count;
  }

  subscribe(cb: () => void): () => void {
    this.#listeners.add(cb);
    return () => this.#listeners.delete(cb);
  }

  /** Binds the badge to a viewer identity and a one-shot own-turn notifier. Idempotent to call
   * more than once (a keep-mounted panel binds once per mount).
   * @param isMine Resolves whether a combatant id belongs to the bound viewer.
   * @param notify Called once per own-turn start. */
  bind(isMine: (combatantId: string) => boolean, notify: () => void): void {
    this.#isMine = isMine;
    this.#notify = notify;
  }

  /** Handles `combat:turn-start`: counts 1 and notifies when bound and the started combatant is
   * the viewer's own; 0 otherwise.
   * @param p The turn-start payload. */
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
   * @param _p The turn-end payload (unused — any turn ending clears the badge). */
  onTurnEnd(_p: CombatTurnEvent): void {
    this.#set(0);
  }

  /** Handles `combat:end`: resets to 0. */
  clear(): void {
    this.#set(0);
  }

  /** Sets the count and notifies listeners only when it actually changed. */
  #set(count: number): void {
    if (this.#count === count) return;
    this.#count = count;
    for (const l of this.#listeners) l();
  }
}
