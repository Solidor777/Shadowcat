import { SvelteSet } from "svelte/reactivity";

/** The set of selected token ids (group-select). A stable instance held by WorldSession and
 * shared via AppContext (the factions panel sets it; the select tool reads + moves it). Backed
 * by a SvelteSet so panel reads are reactive; mutated in place — never reassigned — so the
 * AppContext-captured reference stays valid (the stable-ref rule). */
export class TokenSelection {
  #ids = new SvelteSet<string>();

  get ids(): ReadonlySet<string> {
    return this.#ids;
  }

  has(id: string): boolean {
    return this.#ids.has(id);
  }

  set(ids: Iterable<string>): void {
    this.#ids.clear();
    for (const id of ids) this.#ids.add(id);
  }

  toggle(id: string): void {
    if (!this.#ids.delete(id)) this.#ids.add(id);
  }

  clear(): void {
    this.#ids.clear();
  }
}
