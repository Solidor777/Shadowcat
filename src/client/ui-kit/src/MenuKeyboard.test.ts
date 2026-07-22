import { test, expect, vi } from "vitest";
import { createMenuKeyboard } from "./MenuKeyboard";

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

test("Escape calls onClose with no args (default returnFocus)", () => {
  const items = [mockEl()];
  const onClose = vi.fn();
  const menu = createMenuKeyboard(() => items, onClose);

  menu.handleKeydown(key("Escape"), 0);

  expect(onClose).toHaveBeenCalledOnce();
  expect(onClose).toHaveBeenCalledWith();
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
