import { test, expect, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { PanelsBridge } from "@shadowcat/ui-kit";
import { silentLogger, type PanelMeta } from "@shadowcat/core";
import type { PanelsApi, PanelsChipsView } from "@shadowcat/ui-kit";
import LauncherMenu from "./LauncherMenu.svelte";

afterEach(() => cleanup());

/** A bound bridge whose fake impl records toggle calls and exposes a fixed
 * metaMap — no `module-panels` import (seam boundary). */
function bridgeWith(meta: [string, PanelMeta][]): { bridge: PanelsBridge; toggles: string[] } {
  const toggles: string[] = [];
  const bridge = new PanelsBridge(silentLogger);
  const impl: PanelsApi & PanelsChipsView = {
    open: () => {},
    close: () => {},
    focus: () => {},
    toggle: (id) => toggles.push(id),
    restore: () => {},
    minimized: [],
    metaMap: new Map(meta),
  };
  bridge.bind(impl);
  return { bridge, toggles };
}

const META: [string, PanelMeta][] = [
  ["chat:panel", { icon: "💬", labelKey: "chat.tab" }],
  ["assets:panel", { icon: "🖼️", labelKey: "assets.tab" }],
];

test("the launcher is closed until its trigger is activated", () => {
  const { bridge } = bridgeWith(META);
  render(LauncherMenu, { context: setAppContextForTest({ panels: bridge }) });
  expect(screen.getByTestId("launcher-trigger").getAttribute("aria-expanded")).toBe("false");
  expect(screen.queryByTestId("launcher-menu")).toBeNull();
});

test("opening lists every gmOnly-filtered panel from metaMap as a menuitem", async () => {
  const { bridge } = bridgeWith(META);
  render(LauncherMenu, { context: setAppContextForTest({ panels: bridge }) });
  await fireEvent.click(screen.getByTestId("launcher-trigger"));
  const menu = screen.getByTestId("launcher-menu");
  expect(menu.getAttribute("role")).toBe("menu");
  expect(screen.getByTestId("launcher-item-chat:panel").getAttribute("role")).toBe("menuitem");
  expect(screen.getByTestId("launcher-item-assets:panel")).toBeTruthy();
  expect(screen.getByTestId("launcher-trigger").getAttribute("aria-expanded")).toBe("true");
});

test("activating an item toggles that panel through the bridge and closes the menu", async () => {
  const { bridge, toggles } = bridgeWith(META);
  render(LauncherMenu, { context: setAppContextForTest({ panels: bridge }) });
  await fireEvent.click(screen.getByTestId("launcher-trigger"));
  await fireEvent.click(screen.getByTestId("launcher-item-assets:panel"));
  expect(toggles).toEqual(["assets:panel"]);
  expect(screen.queryByTestId("launcher-menu")).toBeNull();
});

test("Escape on a menu item closes the menu (keyboard path)", async () => {
  const { bridge } = bridgeWith(META);
  render(LauncherMenu, { context: setAppContextForTest({ panels: bridge }) });
  await fireEvent.click(screen.getByTestId("launcher-trigger"));
  await fireEvent.keyDown(screen.getByTestId("launcher-item-chat:panel"), { key: "Escape" });
  expect(screen.queryByTestId("launcher-menu")).toBeNull();
});
