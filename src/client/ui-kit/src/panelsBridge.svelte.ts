// The panel-host bridge (M12a). A stable handle owned by the shell and exposed
// on AppContext, so module/tool components reach the panel host's imperative
// API (open/close/focus/toggle a panel by id) even though the host mounts
// later than callers may first invoke it. Mirrors `SceneInteractionBridge`'s
// late-attachment shape, but the panel host attaches via `bind` (a single
// swap-in, not attach/detach), so calls before `bind` warn once (not silently
// forever) rather than passing an undetected no-op through the whole session.
import type { Logger, PanelMeta } from "@shadowcat/core";

export interface PanelsApi {
  open(id: string): void;
  close(id: string): void;
  focus(id: string): void;
  toggle(id: string): void;
}

/** Read-only live view of the bound panel host's layout, for a surface that
 * renders somewhere else entirely (e.g. the statusbar's `panel-dock` chip
 * strip) — the panel-manager module's `PanelsController` implements this
 * alongside `PanelsApi`, and both are bound in the same `bind()` call. */
export interface PanelsChipsView {
  readonly minimized: readonly string[];
  readonly metaMap: ReadonlyMap<string, PanelMeta>;
  restore(id: string): void;
}

export class PanelsBridge implements PanelsApi, PanelsChipsView {
  // `$state`: a reader that evaluates `minimized`/`metaMap` inside a Svelte
  // `$derived`/template BEFORE `bind()` runs (the panel host mounts later
  // than callers that read AppContext.panels) must still see the bound
  // implementation once it arrives — a plain field carries no reactive
  // signal, so a derived that already ran with `#impl === null` would stay
  // frozen at `[]`/empty forever (buddy-check finding 4).
  #impl = $state<(PanelsApi & PanelsChipsView) | null>(null);
  #warned = false;

  constructor(private readonly logger: Logger) {}

  /** Bind the real panel-host implementation; subsequent calls delegate. */
  bind(impl: PanelsApi & PanelsChipsView): void {
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

  /** Live minimized-panel ids; reads through to the bound controller's `$state`
   * so a caller reading this inside a Svelte `$derived`/template establishes a
   * reactive dependency on layout changes made after `bind()`, from any
   * surface. Empty until bound. */
  get minimized(): readonly string[] {
    return this.#impl?.minimized ?? [];
  }

  /** Live panel metadata map (icon/labelKey), gmOnly-filtered by the bound
   * controller. Empty until bound. */
  get metaMap(): ReadonlyMap<string, PanelMeta> {
    return this.#impl?.metaMap ?? new Map();
  }

  restore(id: string): void {
    if (!this.#impl) return this.#warnOnce();
    this.#impl.restore(id);
  }
}
