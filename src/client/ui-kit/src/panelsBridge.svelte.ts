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

/**
 * Late-binding {@link PanelsApi}/{@link PanelsChipsView} implementation: every call and read
 * delegates to the bound host once {@link PanelsBridge.bind} runs. Before that, the two are
 * treated differently — the imperative METHODS no-op with a one-time logged warning (not a
 * silent forever no-op), while the READS (`minimized`, `metaMap`) return an empty value
 * silently, since an unbound read is the ordinary pre-bind render state rather than a
 * misuse worth warning about.
 */
export class PanelsBridge implements PanelsApi, PanelsChipsView {
  // `$state`: a reader that evaluates `minimized`/`metaMap` inside a Svelte
  // `$derived`/template BEFORE `bind()` runs (the panel host mounts later
  // than callers that read AppContext.panels) must still see the bound
  // implementation once it arrives — a plain field carries no reactive
  // signal, so a derived that already ran with `#impl === null` would stay
  // frozen at `[]`/empty forever (buddy-check finding 4).
  #impl = $state<(PanelsApi & PanelsChipsView) | null>(null);
  #warned = false;

  /** Build an unbound bridge; every call/read no-ops until {@link PanelsBridge.bind} runs.
   * @param logger - Used to emit the one-time "used before bind()" warning.
   * @example new PanelsBridge(logger);
   */
  constructor(private readonly logger: Logger) {}

  /** Bind the real panel-host implementation; subsequent calls delegate.
   * @param impl - The panel-manager module's controller, implementing both interfaces.
   * @example panelsBridge.bind(panelsController);
   */
  bind(impl: PanelsApi & PanelsChipsView): void {
    this.#impl = impl;
  }

  /** Log the "used before bind()" warning exactly once per instance; subsequent calls no-op.
   * @example this.#warnOnce();
   */
  #warnOnce(): void {
    if (this.#warned) return;
    this.#warned = true;
    this.logger.warn("PanelsBridge used before bind(); calls are no-ops until the panel host binds");
  }

  /** Forward to the bound host; warns once (see {@link PanelsBridge.#warnOnce}) and no-ops
   * before `bind()`.
   * @param id - The panel id to open.
   * @returns Nothing.
   * @example panelsBridge.open("chat:panel");
   */
  open(id: string): void {
    if (!this.#impl) return this.#warnOnce();
    this.#impl.open(id);
  }

  /** Forward to the bound host; warns once and no-ops before `bind()`.
   * @param id - The panel id to close.
   * @returns Nothing.
   * @example panelsBridge.close("chat:panel");
   */
  close(id: string): void {
    if (!this.#impl) return this.#warnOnce();
    this.#impl.close(id);
  }

  /** Forward to the bound host; warns once and no-ops before `bind()`.
   * @param id - The panel id to focus.
   * @returns Nothing.
   * @example panelsBridge.focus("chat:panel");
   */
  focus(id: string): void {
    if (!this.#impl) return this.#warnOnce();
    this.#impl.focus(id);
  }

  /** Forward to the bound host; warns once and no-ops before `bind()`.
   * @param id - The panel id to toggle open/closed.
   * @returns Nothing.
   * @example panelsBridge.toggle("chat:panel");
   */
  toggle(id: string): void {
    if (!this.#impl) return this.#warnOnce();
    this.#impl.toggle(id);
  }

  /** Live minimized-panel ids; reads through to the bound controller's `$state`
   * so a caller reading this inside a Svelte `$derived`/template establishes a
   * reactive dependency on layout changes made after `bind()`, from any
   * surface. Empty until bound.
   * @returns The bound host's minimized-panel ids, or `[]` if unbound.
   */
  get minimized(): readonly string[] {
    return this.#impl?.minimized ?? [];
  }

  /** Live panel metadata map (icon/labelKey), gmOnly-filtered by the bound
   * controller. Empty until bound.
   * @returns The bound host's panel metadata map, or an empty map if unbound.
   */
  get metaMap(): ReadonlyMap<string, PanelMeta> {
    return this.#impl?.metaMap ?? new Map();
  }

  /** Forward to the bound host; warns once and no-ops before `bind()`.
   * @param id - The minimized panel id to restore.
   * @returns Nothing.
   * @example panelsBridge.restore("chat:panel");
   */
  restore(id: string): void {
    if (!this.#impl) return this.#warnOnce();
    this.#impl.restore(id);
  }
}
