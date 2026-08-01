/**
 * The actor the place tool will stamp. A stable instance held by WorldSession and shared via
 * AppContext (module-actors sets it; scene-tools reads it). Reactive (`$state`) so a panel can
 * highlight the selection; mutated in place — never reassigned — so the AppContext-captured
 * reference stays valid (the stable-ref rule).
 *
 * Sibling set: this class, {@link TokenSelection}, and `SceneSelection` share a shape at the API
 * level (single stable instance, mutated in place, no pruning when the selected document is
 * deleted — see {@link ActorSelection.selectedId}) but NOT a reactivity mechanism. This class and
 * `SceneSelection` hold a `$state` scalar, gated by its default `===` equality; `TokenSelection`
 * holds a `SvelteSet`, which tracks per-element sources plus an internal version counter. That
 * difference is what produces the divergence below, not a shared backing.
 * Where they diverge: `TokenSelection.set` re-triggers reactivity on any call that starts from a
 * non-empty selection, even when the new ids are identical; `select` here does not (see
 * {@link ActorSelection.select}).
 */
export class ActorSelection {
  #id = $state<string | null>(null);
  #keepAfterPlace = $state(false);

  /** The selected actor's id, or `null` when nothing is selected. Not validated against
   * the document store — an id whose actor is later deleted stays selected; callers that
   * resolve it (e.g. the place tool) must handle a missing document themselves.
   * @returns The selected actor id, or `null`. */
  get selectedId(): string | null {
    return this.#id;
  }

  /**
   * Set (or clear, with `null`) the selected actor. Re-selecting the CURRENTLY selected id
   * (including re-passing `null` when already `null`) is a no-op for reactivity: `$state`'s
   * default equality is reference/value (`===`), so an unchanged assignment does not
   * invalidate readers. Contrast `TokenSelection.set`, which re-triggers on any call that
   * starts from a NON-EMPTY selection (it clears-then-re-adds, and `SvelteSet.clear()` bumps
   * the version whenever it actually removes something) — but not empty→empty, where `clear()`
   * early-returns without bumping.
   * @param id - The actor id to select, or `null` to clear the selection.
   * @example actorSelection.select("actor-1");
   */
  select(id: string | null): void {
    this.#id = id;
  }

  /** User preference: when true, a linked (unique) actor stays selected after placing, so
   * repeated clicks place more linked tokens. Instanced actors always stay selected.
   * @returns The current preference value. */
  get keepAfterPlace(): boolean {
    return this.#keepAfterPlace;
  }

  /**
   * Set the "keep selected after place" preference (see {@link ActorSelection.keepAfterPlace}).
   * @param value - The new preference value.
   * @example actorSelection.setKeepAfterPlace(true);
   */
  setKeepAfterPlace(value: boolean): void {
    this.#keepAfterPlace = value;
  }
}
