/** Default touch hold time before a context menu opens. Deliberately longer than the drag
 * controller's touch arm time, so a touch that begins as a drag-arm never also reads as a
 * context-menu hold. */
export const DEFAULT_CONTEXT_MENU_LONG_PRESS_MS = 500;

/** Pointer travel, in CSS pixels, past which a held touch counts as a scroll or drag rather than
 * a hold. */
export const CONTEXT_MENU_SLOP_PX = 10;

/** One entry of a context menu. */
export interface ContextMenuItem {
  /** Stable id, unique within one menu. */
  id: string;
  /** i18n key of the visible label, resolved by `ContextMenu` at render. */
  labelKey: string;
  /** Runs when the item is chosen; the menu closes first. */
  action: () => void;
  /** Renders the item inert when `true`. */
  disabled?: boolean;
}

/** Where and with what a context menu opens. */
export interface ContextMenuRequest {
  /** Viewport x of the opening gesture (`clientX`). */
  x: number;
  /** Viewport y of the opening gesture (`clientY`). */
  y: number;
  /** The items, read when the gesture fired. */
  items: ContextMenuItem[];
}

/** Options for {@link createContextMenuTrigger}. */
export interface ContextMenuTriggerOptions {
  /** Reads the items at gesture time. An empty list opens no menu and leaves the native menu alone. */
  getItems: () => ContextMenuItem[];
  /** Receives the request to open; the caller renders `ContextMenu` from it. */
  onOpen: (request: ContextMenuRequest) => void;
  /** Touch hold time before the menu opens. Defaults to {@link DEFAULT_CONTEXT_MENU_LONG_PRESS_MS}. */
  longPressMs?: number;
  /** Consulted when a gesture fires; `false` suppresses the menu. A row that also carries a drag
   * handle passes `() => drag.dragging === null` so an in-progress drag never also opens a menu. */
  canOpen?: () => boolean;
}

/** The event handlers a caller spreads onto its trigger element. */
export interface ContextMenuTrigger {
  /** Right-click: opens immediately at the pointer and cancels the native menu. */
  oncontextmenu: (e: MouseEvent) => void;
  /** Touch: arms the hold timer. Other pointer types never arm it. */
  onpointerdown: (e: PointerEvent) => void;
  /** Touch: disarms the hold timer when the finger lifts early. */
  onpointerup: (e: PointerEvent) => void;
  /** Touch: disarms the hold timer when the system cancels the pointer. */
  onpointercancel: (e: PointerEvent) => void;
  /** Touch: disarms the hold timer once the pointer travels past {@link CONTEXT_MENU_SLOP_PX}. */
  onpointermove: (e: PointerEvent) => void;
}

/**
 * Builds the gesture handlers that open a context menu: right-click opens at once, and a touch
 * held still for `longPressMs` opens at the touch point. The handlers only report a
 * {@link ContextMenuRequest}; rendering and keyboard behavior belong to `ContextMenu`.
 * @param opts - The item source, the open callback and the optional hold time and guard.
 * @returns Handlers to spread onto the trigger element.
 * @example
 * const trigger = createContextMenuTrigger({
 *   getItems: () => [{ id: "open", labelKey: "notes.tab", action: () => {} }],
 *   onOpen: (request) => void request,
 * });
 * trigger.oncontextmenu(new MouseEvent("contextmenu", { clientX: 4, clientY: 8 }));
 */
export function createContextMenuTrigger(opts: ContextMenuTriggerOptions): ContextMenuTrigger {
  const longPressMs = opts.longPressMs ?? DEFAULT_CONTEXT_MENU_LONG_PRESS_MS;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let startX = 0;
  let startY = 0;

  /**
   * Opens the menu at a point unless the guard or an empty item list forbids it.
   * @param x - Viewport x.
   * @param y - Viewport y.
   * @returns `true` when a menu was requested.
   * @example
   * open(4, 8);
   */
  function open(x: number, y: number): boolean {
    if (opts.canOpen && !opts.canOpen()) return false;
    const items = opts.getItems();
    if (items.length === 0) return false;
    opts.onOpen({ x, y, items });
    return true;
  }

  /**
   * Clears the armed hold timer, if any.
   * @example
   * disarm();
   */
  function disarm(): void {
    if (timer !== null) clearTimeout(timer);
    timer = null;
  }

  return {
    oncontextmenu(e) {
      if (open(e.clientX, e.clientY)) e.preventDefault();
    },
    onpointerdown(e) {
      disarm();
      if (e.pointerType !== "touch") return;
      startX = e.clientX;
      startY = e.clientY;
      timer = setTimeout(() => {
        timer = null;
        open(startX, startY);
      }, longPressMs);
    },
    onpointerup: disarm,
    onpointercancel: disarm,
    onpointermove(e) {
      if (timer === null) return;
      if (Math.hypot(e.clientX - startX, e.clientY - startY) > CONTEXT_MENU_SLOP_PX) disarm();
    },
  };
}
