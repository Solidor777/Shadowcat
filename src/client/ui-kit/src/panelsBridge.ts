// The panel-host bridge (M12a). A stable handle owned by the shell and exposed
// on AppContext, so module/tool components reach the panel host's imperative
// API (open/close/focus/toggle a panel by id) even though the host mounts
// later than callers may first invoke it. Mirrors `SceneInteractionBridge`'s
// late-attachment shape, but the panel host attaches via `bind` (a single
// swap-in, not attach/detach), so calls before `bind` warn once (not silently
// forever) rather than passing an undetected no-op through the whole session.
import type { Logger } from "@shadowcat/core";

export interface PanelsApi {
  open(id: string): void;
  close(id: string): void;
  focus(id: string): void;
  toggle(id: string): void;
}

export class PanelsBridge implements PanelsApi {
  #impl: PanelsApi | null = null;
  #warned = false;

  constructor(private readonly logger: Logger) {}

  /** Bind the real panel-host implementation; subsequent calls delegate. */
  bind(impl: PanelsApi): void {
    this.#impl = impl;
  }

  #warnOnce(): void {
    if (this.#warned) return;
    this.#warned = true;
    this.logger.warn("PanelsBridge used before bind(); calls are no-ops until the panel host binds");
  }

  open(id: string): void {
    if (!this.#impl) return this.#warnOnce();
    this.#impl.open(id);
  }

  close(id: string): void {
    if (!this.#impl) return this.#warnOnce();
    this.#impl.close(id);
  }

  focus(id: string): void {
    if (!this.#impl) return this.#warnOnce();
    this.#impl.focus(id);
  }

  toggle(id: string): void {
    if (!this.#impl) return this.#warnOnce();
    this.#impl.toggle(id);
  }
}
