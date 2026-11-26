/** Priority bands for {@link EscapeCoordinator.register}. A higher number wins; the bands are a
 * convention, not a closed set, so a surface may register at any number. */
export const ESCAPE_PRIORITY = {
  /** Scene-tool cancel (a tool rail's active editor); loses to every dismissible surface. */
  TOOL: 50,
  /** Menu popups driven by `createMenuKeyboard`. */
  MENU: 100,
  /** Hover/focus tooltips and popovers. */
  TOOLTIP: 150,
  /** Inline overlays: asset picker, folder-tree dialogs, preview confirms. */
  OVERLAY: 200,
  /** A floating panel's dialog, above inline overlays and below modals. */
  FLOATING_PANEL: 250,
  /** Blocking modals: confirm/prompt dialogs, the merge-conflict modal. */
  MODAL: 300,
} as const;

/** One registered dismissible surface. */
interface EscapeRegistration {
  /** The surface's priority; a higher value wins. */
  priority: number;
  /** Registration order; the larger value is the more recent registration. */
  order: number;
  /** Closes the surface. */
  onEscape: () => void;
  /** When present, the registration competes only for Escape events it accepts. */
  appliesTo?: (event: KeyboardEvent) => boolean;
}

/**
 * The single decider for which dismissible surface an Escape keystroke closes. One capture-phase
 * `window` keydown listener is installed at construction; every dismissible surface registers a
 * close callback with a numeric priority instead of listening itself, so two open surfaces never
 * both react to one keystroke.
 *
 * The highest priority wins; equal priorities resolve most-recently-registered first. The
 * coordinator calls `preventDefault` and `stopPropagation` on the native event before invoking the
 * winner, so a callback receives no event and no downstream handler sees a consumed Escape. With
 * no registration the event is left untouched.
 *
 * Priority bands live in {@link ESCAPE_PRIORITY}. A registered callback owns what "close" means to
 * its surface (focus return, cancel versus commit); the coordinator only picks which one runs.
 * @example
 * const dispose = escapeCoordinator.register(ESCAPE_PRIORITY.OVERLAY, () => closeOverlay());
 * dispose();
 */
export class EscapeCoordinator {
  /** Live registrations, in registration order. */
  #registrations: EscapeRegistration[] = [];
  /** Monotonic registration counter backing the most-recent-wins tie-break. */
  #counter = 0;

  /**
   * Installs the capture-phase Escape listener on `target` for the coordinator's lifetime.
   * @param target - The event target to listen on; defaults to the global `window` and installs
   * nothing when no window exists (a node-environment import).
   * @example
   * const coordinator = new EscapeCoordinator(window);
   */
  constructor(target: EventTarget | undefined = typeof window === "undefined" ? undefined : window) {
    target?.addEventListener("keydown", (event) => this.#onKeydown(event as KeyboardEvent), true);
  }

  /**
   * Registers a surface's close callback.
   * @param priority - Higher wins; see {@link ESCAPE_PRIORITY}.
   * @param onEscape - Closes the surface; runs only when this registration is the winner.
   * @param appliesTo - Optional filter on the native event. A registration whose filter returns
   * false is skipped for that keystroke, so a surface scoped to part of the page (a floating
   * panel's own dialog, the canvas) never consumes an Escape aimed elsewhere.
   * @returns A disposer that removes the registration; calling it again is a no-op.
   * @example
   * const dispose = escapeCoordinator.register(ESCAPE_PRIORITY.MENU, () => close());
   */
  register(priority: number, onEscape: () => void, appliesTo?: (event: KeyboardEvent) => boolean): () => void {
    const entry: EscapeRegistration = { priority, order: this.#counter++, onEscape, appliesTo };
    this.#registrations.push(entry);
    return () => {
      const at = this.#registrations.indexOf(entry);
      if (at >= 0) this.#registrations.splice(at, 1);
    };
  }

  /**
   * Handles one keydown: ignores everything but Escape, and for Escape runs the winning
   * registration after consuming the native event.
   * @param event - The native keydown event.
   * @example
   * // private; installed as the capture-phase listener in the constructor
   */
  #onKeydown(event: KeyboardEvent): void {
    // Escape during an IME composition cancels the composition, not the surface; the IME owns it.
    if (event.key !== "Escape" || event.isComposing) return;
    let winner: EscapeRegistration | undefined;
    for (const r of this.#registrations) {
      if (r.appliesTo !== undefined && !r.appliesTo(event)) continue;
      if (winner === undefined || r.priority > winner.priority || (r.priority === winner.priority && r.order > winner.order)) {
        winner = r;
      }
    }
    if (winner === undefined) return;
    event.preventDefault();
    event.stopPropagation();
    winner.onEscape();
  }
}

/** The shell-wide coordinator; every dismissible surface registers here. */
export const escapeCoordinator = new EscapeCoordinator();
