import { test, expect, vi, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/svelte";
import PanelMenu from "./PanelMenu.svelte";

afterEach(() => cleanup());

/** The `onClose` wrapper closure must re-read the live `onClose` prop on
 * every keydown, not a value captured once at mount. */
test("Escape on a menu item calls the current onClose prop (keyboard-wiring closure)", async () => {
  const onClose = vi.fn();
  render(PanelMenu, { props: { onCommand: () => {}, onClose } });

  await fireEvent.keyDown(screen.getByTestId("panel-menu-dockRight"), { key: "Escape" });

  expect(onClose).toHaveBeenCalledTimes(1);
  expect(onClose).toHaveBeenCalledWith(undefined);
});

/** A replaced `onClose` prop after mount must be the one invoked — proves
 * the closure reads the prop live rather than a reference captured once. */
test("updating the onClose prop after mount is honored by the same keyboard wiring", async () => {
  const firstOnClose = vi.fn();
  const { rerender } = render(PanelMenu, { props: { onCommand: () => {}, onClose: firstOnClose } });

  const secondOnClose = vi.fn();
  await rerender({ onCommand: () => {}, onClose: secondOnClose });

  await fireEvent.keyDown(screen.getByTestId("panel-menu-dockRight"), { key: "Escape" });

  expect(secondOnClose).toHaveBeenCalledTimes(1);
  expect(firstOnClose).not.toHaveBeenCalled();
});
