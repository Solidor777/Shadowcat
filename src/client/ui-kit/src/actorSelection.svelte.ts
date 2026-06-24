// The actor the place tool will stamp. A stable instance held by WorldSession and shared via
// AppContext (module-actors sets it; scene-tools reads it). Reactive ($state) so a panel can
// highlight the selection; mutated in place — never reassigned — so the AppContext-captured
// reference stays valid (the stable-ref rule).
export class ActorSelection {
  #id = $state<string | null>(null);

  get selectedId(): string | null {
    return this.#id;
  }

  select(id: string | null): void {
    this.#id = id;
  }
}
