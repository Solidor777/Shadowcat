import { test, expect, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { PanelsBridge } from "@shadowcat/ui-kit";
import { silentLogger } from "@shadowcat/core";
import type { PanelsApi, PanelsChipsView } from "@shadowcat/ui-kit";
import TopBar from "./TopBar.svelte";

afterEach(() => cleanup());

function boundBridge(): { bridge: PanelsBridge; toggles: string[] } {
  const toggles: string[] = [];
  const bridge = new PanelsBridge(silentLogger);
  const impl: PanelsApi & PanelsChipsView = {
    open: () => {},
    close: () => {},
    focus: () => {},
    toggle: (id) => toggles.push(id),
    restore: () => {},
    minimized: [],
    metaMap: new Map([["chat:panel", { icon: "💬", labelKey: "chat.tab" }]]),
  };
  bridge.bind(impl);
  return { bridge, toggles };
}

test("shows the launcher, the world title, presence, and a settings entry", () => {
  const { bridge } = boundBridge();
  render(TopBar, {
    context: setAppContextForTest({ world: "Rivertown", panels: bridge, members: new Map([["u1", "Ada"]]) }),
  });
  expect(screen.getByTestId("launcher-trigger")).toBeTruthy();
  expect(screen.getByTestId("presence")).toBeTruthy();
  expect(screen.getByTestId("topbar-settings")).toBeTruthy();
  // World title text uses the topbar.world key (test `t` echoes keys).
  expect(screen.getByTestId("topbar-title").textContent).toContain("topbar.world");
});

test("the settings entry toggles the settings panel through the bridge", async () => {
  const { bridge, toggles } = boundBridge();
  render(TopBar, { context: setAppContextForTest({ panels: bridge }) });
  await fireEvent.click(screen.getByTestId("topbar-settings"));
  expect(toggles).toEqual(["settings:panel"]);
});
