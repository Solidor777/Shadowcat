// The actor the place tool will stamp. A stable instance held by WorldSession and shared via
// AppContext (module-actors sets it; scene-tools reads it). Reactive ($state) so a panel can
// highlight the selection; mutated in place — never reassigned — so the AppContext-captured
// reference stays valid (the stable-ref rule).
export class ActorSelection {
  #id = $state<string | null>(null);
  #keepAfterPlace = $state(false);

  get selectedId(): string | null {
    return this.#id;
  }

  select(id: string | null): void {
    this.#id = id;
  }

  /** User preference: when true, a linked (unique) actor stays selected after placing, so
   * repeated clicks place more linked tokens. Instanced actors always stay selected. */
  get keepAfterPlace(): boolean {
    return this.#keepAfterPlace;
  }

  setKeepAfterPlace(value: boolean): void {
    this.#keepAfterPlace = value;
  }
}
