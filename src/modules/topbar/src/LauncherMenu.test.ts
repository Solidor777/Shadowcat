import { test, expect, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/svelte";
import { tick } from "svelte";
import { SvelteMap } from "svelte/reactivity";
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

test("Escape on a menu item returns focus to the trigger", async () => {
  const { bridge } = bridgeWith(META);
  render(LauncherMenu, { context: setAppContextForTest({ panels: bridge }) });
  await fireEvent.click(screen.getByTestId("launcher-trigger"));
  await fireEvent.keyDown(screen.getByTestId("launcher-item-chat:panel"), { key: "Escape" });
  await Promise.resolve();
  expect(document.activeElement).toBe(screen.getByTestId("launcher-trigger"));
});

test("Tab on a menu item closes the menu but does NOT force focus back to the trigger (APG Menu Button pattern)", async () => {
  const { bridge } = bridgeWith(META);
  render(LauncherMenu, { context: setAppContextForTest({ panels: bridge }) });
  const trigger = screen.getByTestId("launcher-trigger");
  await fireEvent.click(trigger);
  const item = screen.getByTestId("launcher-item-chat:panel");
  const event = await fireEvent.keyDown(item, { key: "Tab" });
  // Native Tab traversal is not intercepted: preventDefault must not be called.
  expect(event).toBe(true);
  await Promise.resolve();
  expect(screen.queryByTestId("launcher-menu")).toBeNull();
  // jsdom does not natively advance focus for an unprevented Tab keydown, so
  // this only asserts the menu did not force focus back onto the trigger.
  expect(document.activeElement).not.toBe(trigger);
});

test("Enter on the trigger while the menu is open closes it (true toggle)", async () => {
  const { bridge } = bridgeWith(META);
  render(LauncherMenu, { context: setAppContextForTest({ panels: bridge }) });
  const trigger = screen.getByTestId("launcher-trigger");
  await fireEvent.click(trigger);
  expect(screen.queryByTestId("launcher-menu")).not.toBeNull();
  await fireEvent.keyDown(trigger, { key: "Enter" });
  expect(screen.queryByTestId("launcher-menu")).toBeNull();
});

/** A bound bridge over a live `SvelteMap`, so mutating the returned map in place after render is
 * observed by `LauncherMenu`'s `panels = $derived(...)` — a plain `Map` (as `bridgeWith` above
 * builds) does NOT trigger that re-derive on an in-place `.delete()`, only on `PanelsBridge`
 * rebinding to a different `metaMap` reference entirely. Empirically confirmed while writing the
 * two tests below: swapping this back to a plain `Map` makes both fail (the menu never reacts to
 * the mutation), so the `SvelteMap` is load-bearing, not incidental.
 * @param meta The initial `[id, PanelMeta]` entries.
 * @returns The bound bridge plus the live `SvelteMap` backing its `metaMap`.
 * @example
 * ```
 * // private test helper; not part of the public API
 * const { bridge, live } = reactiveBridgeWith(META);
 * ```
 */
function reactiveBridgeWith(meta: [string, PanelMeta][]): { bridge: PanelsBridge; live: SvelteMap<string, PanelMeta> } {
  const live = new SvelteMap(meta);
  const bridge = new PanelsBridge(silentLogger);
  const impl: PanelsApi & PanelsChipsView = {
    open: () => {},
    close: () => {},
    focus: () => {},
    toggle: () => {},
    restore: () => {},
    minimized: [],
    metaMap: live,
  };
  bridge.bind(impl);
  return { bridge, live };
}

test("a live module unload removing the FOCUSED item's panel closes the menu and recovers focus to the trigger", async () => {
  const { bridge, live } = reactiveBridgeWith(META);
  render(LauncherMenu, { context: setAppContextForTest({ panels: bridge }) });
  const trigger = screen.getByTestId("launcher-trigger");
  await fireEvent.click(trigger);
  const chatItem = screen.getByTestId("launcher-item-chat:panel");
  chatItem.focus();
  expect(document.activeElement).toBe(chatItem);

  // Simulates a live module uninstall dropping the focused item's panel out of metaMap.
  live.delete("chat:panel");
  await tick();

  expect(screen.queryByTestId("launcher-menu")).toBeNull();
  expect(document.activeElement).toBe(trigger);
  expect(document.activeElement).not.toBe(document.body);
});

test("removing a DIFFERENT (non-focused) item's panel leaves the menu open and focus untouched", async () => {
  const { bridge, live } = reactiveBridgeWith(META);
  render(LauncherMenu, { context: setAppContextForTest({ panels: bridge }) });
  await fireEvent.click(screen.getByTestId("launcher-trigger"));
  const chatItem = screen.getByTestId("launcher-item-chat:panel");
  chatItem.focus();
  expect(document.activeElement).toBe(chatItem);

  // A different item's panel disappears; focus never moves to <body>, so the narrow
  // "focused item disappeared" condition must not fire.
  live.delete("assets:panel");
  await tick();

  expect(screen.queryByTestId("launcher-menu")).not.toBeNull();
  expect(document.activeElement).toBe(chatItem);
});

test("the trigger's aria-controls references the open menu's id", async () => {
  const { bridge } = bridgeWith(META);
  render(LauncherMenu, { context: setAppContextForTest({ panels: bridge }) });
  const trigger = screen.getByTestId("launcher-trigger");
  expect(trigger.getAttribute("aria-controls")).toBeNull();
  await fireEvent.click(trigger);
  const menu = screen.getByTestId("launcher-menu");
  expect(trigger.getAttribute("aria-controls")).toBe(menu.id);
  expect(menu.id).toBeTruthy();
});
