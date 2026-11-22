import { describe, it, expect, afterEach, beforeEach, vi } from "vitest";
import {
  createContextMenuTrigger,
  CONTEXT_MENU_SLOP_PX,
  DEFAULT_CONTEXT_MENU_LONG_PRESS_MS,
  type ContextMenuItem,
} from "./contextMenu";

const ITEMS: ContextMenuItem[] = [{ id: "a", labelKey: "notes.open", action: () => {} }];

/** A pointer event stand-in carrying only the fields the trigger reads. */
function pointer(type: string, x: number, y: number, pointerType = "touch"): PointerEvent {
  return { type, clientX: x, clientY: y, pointerType } as PointerEvent;
}

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe("createContextMenuTrigger", () => {
  it("right-click opens at once at the pointer and cancels the native menu", () => {
    const onOpen = vi.fn();
    const trigger = createContextMenuTrigger({ getItems: () => ITEMS, onOpen });
    const event = new MouseEvent("contextmenu", { clientX: 12, clientY: 34, cancelable: true });

    trigger.oncontextmenu(event);

    expect(onOpen).toHaveBeenCalledWith({ x: 12, y: 34, items: ITEMS });
    expect(event.defaultPrevented).toBe(true);
  });

  it("an empty item list opens nothing and leaves the native menu alone", () => {
    const onOpen = vi.fn();
    const trigger = createContextMenuTrigger({ getItems: () => [], onOpen });
    const event = new MouseEvent("contextmenu", { cancelable: true });

    trigger.oncontextmenu(event);

    expect(onOpen).not.toHaveBeenCalled();
    expect(event.defaultPrevented).toBe(false);
  });

  it("a touch held past longPressMs opens at the touch point", () => {
    const onOpen = vi.fn();
    const trigger = createContextMenuTrigger({ getItems: () => ITEMS, onOpen });

    trigger.onpointerdown(pointer("pointerdown", 5, 6));
    vi.advanceTimersByTime(DEFAULT_CONTEXT_MENU_LONG_PRESS_MS - 1);
    expect(onOpen).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);

    expect(onOpen).toHaveBeenCalledWith({ x: 5, y: 6, items: ITEMS });
  });

  it("honours a custom longPressMs", () => {
    const onOpen = vi.fn();
    const trigger = createContextMenuTrigger({ getItems: () => ITEMS, onOpen, longPressMs: 80 });

    trigger.onpointerdown(pointer("pointerdown", 0, 0));
    vi.advanceTimersByTime(80);

    expect(onOpen).toHaveBeenCalledOnce();
  });

  it("a touch released early does not open (dropping the pointerup guard opens on every tap)", () => {
    const onOpen = vi.fn();
    const trigger = createContextMenuTrigger({ getItems: () => ITEMS, onOpen });

    trigger.onpointerdown(pointer("pointerdown", 0, 0));
    vi.advanceTimersByTime(100);
    trigger.onpointerup(pointer("pointerup", 0, 0));
    vi.advanceTimersByTime(1000);

    expect(onOpen).not.toHaveBeenCalled();
  });

  it("a touch cancelled by the system does not open", () => {
    const onOpen = vi.fn();
    const trigger = createContextMenuTrigger({ getItems: () => ITEMS, onOpen });

    trigger.onpointerdown(pointer("pointerdown", 0, 0));
    trigger.onpointercancel(pointer("pointercancel", 0, 0));
    vi.advanceTimersByTime(1000);

    expect(onOpen).not.toHaveBeenCalled();
  });

  it("a touch that moves past the slop radius does not open (dropping the slop guard opens on every scroll)", () => {
    const onOpen = vi.fn();
    const trigger = createContextMenuTrigger({ getItems: () => ITEMS, onOpen });

    trigger.onpointerdown(pointer("pointerdown", 0, 0));
    trigger.onpointermove(pointer("pointermove", 0, CONTEXT_MENU_SLOP_PX + 1));
    vi.advanceTimersByTime(1000);

    expect(onOpen).not.toHaveBeenCalled();
  });

  it("a touch that stays inside the slop radius still opens", () => {
    const onOpen = vi.fn();
    const trigger = createContextMenuTrigger({ getItems: () => ITEMS, onOpen });

    trigger.onpointerdown(pointer("pointerdown", 0, 0));
    trigger.onpointermove(pointer("pointermove", 3, 3));
    vi.advanceTimersByTime(DEFAULT_CONTEXT_MENU_LONG_PRESS_MS);

    expect(onOpen).toHaveBeenCalledOnce();
  });

  it("a mouse or pen press never arms the hold", () => {
    const onOpen = vi.fn();
    const trigger = createContextMenuTrigger({ getItems: () => ITEMS, onOpen });

    trigger.onpointerdown(pointer("pointerdown", 0, 0, "mouse"));
    trigger.onpointerdown(pointer("pointerdown", 0, 0, "pen"));
    vi.advanceTimersByTime(1000);

    expect(onOpen).not.toHaveBeenCalled();
  });

  it("canOpen=false suppresses both gestures (an in-progress drag never also opens a menu)", () => {
    const onOpen = vi.fn();
    const trigger = createContextMenuTrigger({ getItems: () => ITEMS, onOpen, canOpen: () => false });
    const event = new MouseEvent("contextmenu", { cancelable: true });

    trigger.oncontextmenu(event);
    trigger.onpointerdown(pointer("pointerdown", 0, 0));
    vi.advanceTimersByTime(1000);

    expect(onOpen).not.toHaveBeenCalled();
    expect(event.defaultPrevented).toBe(false);
  });

  it("canOpen is read when the hold fires, not when the press starts", () => {
    const onOpen = vi.fn();
    let dragging = false;
    const trigger = createContextMenuTrigger({ getItems: () => ITEMS, onOpen, canOpen: () => !dragging });

    trigger.onpointerdown(pointer("pointerdown", 0, 0));
    dragging = true;
    vi.advanceTimersByTime(DEFAULT_CONTEXT_MENU_LONG_PRESS_MS);

    expect(onOpen).not.toHaveBeenCalled();
  });
});
