import { test, expect, vi } from "vitest";
import { createMenuKeyboard } from "./MenuKeyboard";
import { escapeCoordinator, ESCAPE_PRIORITY } from "./escapeCoordinator";

function mockEl(): HTMLElement {
  const el = document.createElement("button");
  document.body.appendChild(el);
  return el;
}

function key(key: string): KeyboardEvent {
  return new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true });
}

test("ArrowDown moves focus to the next item, wrapping past the last", () => {
  const items = [mockEl(), mockEl(), mockEl()];
  const onClose = vi.fn();
  const menu = createMenuKeyboard(() => items, onClose);

  menu.focusItem(2);
  menu.handleKeydown(key("ArrowDown"), 2);

  expect(document.activeElement).toBe(items[0]); // wraps from index 2 to 0
});

test("ArrowUp moves focus to the previous item, wrapping before the first", () => {
  const items = [mockEl(), mockEl(), mockEl()];
  const menu = createMenuKeyboard(() => items, vi.fn());

  menu.handleKeydown(key("ArrowUp"), 0);

  expect(document.activeElement).toBe(items[2]);
});

test("handleKeydown ignores Escape: it reaches the menu only through the coordinator", () => {
  const items = [mockEl()];
  const onClose = vi.fn();
  const menu = createMenuKeyboard(() => items, onClose);
  const event = key("Escape");

  menu.handleKeydown(event, 0);

  expect(onClose).not.toHaveBeenCalled();
  expect(event.defaultPrevented).toBe(false);
});

test("activate registers exactly one MENU-priority escape callback that closes with no args", () => {
  const register = vi.spyOn(escapeCoordinator, "register");
  const onClose = vi.fn();
  const menu = createMenuKeyboard(() => [], onClose);

  menu.activate();
  menu.activate();

  expect(register).toHaveBeenCalledTimes(1);
  expect(register.mock.calls[0]?.[0]).toBe(ESCAPE_PRIORITY.MENU);
  window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
  expect(onClose).toHaveBeenCalledOnce();
  expect(onClose).toHaveBeenCalledWith();
  menu.deactivate();
  register.mockRestore();
});

test("deactivate removes the registration and is idempotent", () => {
  const onClose = vi.fn();
  const menu = createMenuKeyboard(() => [], onClose);

  menu.activate();
  menu.deactivate();
  menu.deactivate();
  window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));

  expect(onClose).not.toHaveBeenCalled();
});

test("Tab calls onClose(false)", () => {
  const items = [mockEl()];
  const onClose = vi.fn();
  const menu = createMenuKeyboard(() => items, onClose);

  menu.handleKeydown(key("Tab"), 0);

  expect(onClose).toHaveBeenCalledWith(false);
});

test("Home focuses the first item, End focuses the last", () => {
  const items = [mockEl(), mockEl(), mockEl()];
  const menu = createMenuKeyboard(() => items, vi.fn());

  menu.handleKeydown(key("Home"), 1);
  expect(document.activeElement).toBe(items[0]);

  menu.handleKeydown(key("End"), 1);
  expect(document.activeElement).toBe(items[2]);
});

test("focusItem is a no-op when there are no items", () => {
  const menu = createMenuKeyboard(() => [], vi.fn());
  expect(() => menu.focusItem(0)).not.toThrow();
});
