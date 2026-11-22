import { describe, it, expect, afterEach, vi } from "vitest";
import { render, fireEvent, cleanup, screen } from "@testing-library/svelte";
import * as menuKeyboardModule from "./MenuKeyboard";
import { escapeCoordinator, ESCAPE_PRIORITY } from "./escapeCoordinator";
import ContextMenu from "./ContextMenu.svelte";
import type { ContextMenuItem } from "./contextMenu";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

/** Two enabled items and one disabled item, with spies for their actions. */
function items(): { list: ContextMenuItem[]; open: ReturnType<typeof vi.fn>; del: ReturnType<typeof vi.fn> } {
  const open = vi.fn();
  const del = vi.fn();
  return {
    list: [
      { id: "open", labelKey: "notes.open", action: open },
      { id: "delete", labelKey: "notes.delete", action: del },
      { id: "off", labelKey: "notes.moveTo", action: vi.fn(), disabled: true },
    ],
    open,
    del,
  };
}

describe("ContextMenu", () => {
  it("renders one menuitem per item and focuses the first enabled one", () => {
    const { list } = items();
    render(ContextMenu, { props: { items: list, x: 10, y: 10, onClose: () => {} } });

    expect(screen.getAllByRole("menuitem")).toHaveLength(3);
    expect(document.activeElement).toBe(screen.getByTestId("context-menu-item-open"));
  });

  it("choosing an item closes the menu and then runs its action", () => {
    const { list, open } = items();
    const order: string[] = [];
    const onClose = vi.fn(() => order.push("close"));
    open.mockImplementation(() => order.push("action"));
    render(ContextMenu, { props: { items: list, x: 0, y: 0, onClose } });

    screen.getByTestId("context-menu-item-open").click();

    expect(order).toEqual(["close", "action"]);
  });

  it("a disabled item neither closes the menu nor runs", async () => {
    const { list } = items();
    const onClose = vi.fn();
    render(ContextMenu, { props: { items: list, x: 0, y: 0, onClose } });

    await fireEvent.click(screen.getByTestId("context-menu-item-off"));

    expect(onClose).not.toHaveBeenCalled();
  });

  it("Escape closes the menu through the coordinator at the menu priority", () => {
    const register = vi.spyOn(escapeCoordinator, "register");
    const onClose = vi.fn();
    render(ContextMenu, { props: { items: items().list, x: 0, y: 0, onClose } });

    expect(register.mock.calls[0]?.[0]).toBe(ESCAPE_PRIORITY.MENU);
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));

    expect(onClose).toHaveBeenCalledOnce();
  });

  it("closing drops the Escape registration", () => {
    const onClose = vi.fn();
    const view = render(ContextMenu, { props: { items: items().list, x: 0, y: 0, onClose } });
    view.unmount();

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));

    expect(onClose).not.toHaveBeenCalled();
  });

  it("keyboard navigation is delegated to createMenuKeyboard, not reimplemented", async () => {
    const spy = vi.spyOn(menuKeyboardModule, "createMenuKeyboard");
    render(ContextMenu, { props: { items: items().list, x: 0, y: 0, onClose: () => {} } });

    expect(spy).toHaveBeenCalledOnce();
    await fireEvent.keyDown(screen.getByTestId("context-menu-item-open"), { key: "ArrowDown" });
    expect(document.activeElement).toBe(screen.getByTestId("context-menu-item-delete"));
    // The disabled last item cannot hold focus, so End lands on the last enabled one.
    await fireEvent.keyDown(screen.getByTestId("context-menu-item-open"), { key: "End" });
    expect(document.activeElement).toBe(screen.getByTestId("context-menu-item-delete"));
  });

  it("Tab closes without returning focus", async () => {
    const opener = document.createElement("button");
    document.body.appendChild(opener);
    opener.focus();
    const onClose = vi.fn();
    render(ContextMenu, { props: { items: items().list, x: 0, y: 0, onClose } });

    await fireEvent.keyDown(screen.getByTestId("context-menu-item-open"), { key: "Tab" });

    expect(onClose).toHaveBeenCalledOnce();
    expect(document.activeElement).not.toBe(opener);
    opener.remove();
  });

  it("Escape returns focus to the element that held it when the menu opened", () => {
    const opener = document.createElement("button");
    document.body.appendChild(opener);
    opener.focus();
    render(ContextMenu, { props: { items: items().list, x: 0, y: 0, onClose: () => {} } });

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));

    expect(document.activeElement).toBe(opener);
    opener.remove();
  });

  it("an outside press closes the menu without returning focus", async () => {
    const onClose = vi.fn();
    render(ContextMenu, { props: { items: items().list, x: 0, y: 0, onClose } });

    await fireEvent.pointerDown(screen.getByTestId("context-menu-backdrop"));

    expect(onClose).toHaveBeenCalledOnce();
  });

  it("positions at the gesture point", () => {
    render(ContextMenu, { props: { items: items().list, x: 40, y: 50, onClose: () => {} } });

    const menu = screen.getByTestId("context-menu");
    expect(menu.style.left).toBe("40px");
    expect(menu.style.top).toBe("50px");
  });
});
