import { SvelteSet } from "svelte/reactivity";

/**
 * The set of selected token ids (group-select). A stable instance held by WorldSession and
 * shared via AppContext (the factions panel sets it; the select tool reads + moves it). Backed
 * by a SvelteSet so panel reads are reactive; mutated in place — never reassigned — so the
 * AppContext-captured reference stays valid (the stable-ref rule).
 *
 * Sibling of `ActorSelection`/`SceneSelection`: same stable-instance/mutate-in-place shape, and
 * likewise does not prune an id whose token is later deleted — a stale id stays selected until
 * something calls `toggle`/`set`/`clear`; consumers (e.g. scene-tools) resolve against the
 * current document store and handle a miss themselves. Diverges on repeat-selection reactivity:
 * see {@link TokenSelection.set}.
 */
export class TokenSelection {
  #ids = new SvelteSet<string>();

  /** The currently selected token ids. Read-only view over the live set — iterate or check
   * `.has`/`.size` inside an effect/derived to track future changes.
   * @returns The selected id set. */
  get ids(): ReadonlySet<string> {
    return this.#ids;
  }

  /** Whether `id` is currently selected.
   * @param id - The token id to check.
   * @returns `true` if selected.
   * @example tokenSelection.has("tok1");
   */
  has(id: string): boolean {
    return this.#ids.has(id);
  }

  /**
   * Replace the whole selection with `ids`. Unlike `ActorSelection.select`/`SceneSelection.select`,
   * this is NOT a no-op when the new set is identical to the current one: it always clears then
   * re-adds (`SvelteSet.clear`/`.add`), which increments the set's internal version on every call
   * that starts non-empty — so a caller passing back the same ids still re-triggers every reader
   * (effect/derived) subscribed to `.ids`/`.has`/`.size`.
   * @param ids - The token ids to select; replaces the previous selection entirely.
   * @example tokenSelection.set(["tok1", "tok2"]);
   */
  set(ids: Iterable<string>): void {
    this.#ids.clear();
    for (const id of ids) this.#ids.add(id);
  }

  /** Flip `id`'s membership: select it if unselected, deselect it if selected.
   * @param id - The token id to toggle.
   * @example tokenSelection.toggle("tok1");
   */
  toggle(id: string): void {
    if (!this.#ids.delete(id)) this.#ids.add(id);
  }

  /** Deselect every token. A no-op (no reactivity triggered) when already empty.
   * @example tokenSelection.clear();
   */
  clear(): void {
    this.#ids.clear();
  }
}
