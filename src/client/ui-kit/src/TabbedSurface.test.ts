import { render, screen, fireEvent } from "@testing-library/svelte";
import { test, expect } from "vitest";
import { ContributionRegistry } from "@shadowcat/core";
import { setAppContextForTest } from "./__fixtures__/appContextTest";
import TabbedSurface from "./TabbedSurface.svelte";
import Probe from "./__fixtures__/Probe.svelte";

// Translation stub that echoes the key so assertions can check for it directly.
const t = (key: string) => key;

function registryWithTabs(): ContributionRegistry {
  const registry = new ContributionRegistry();
  registry.contribute({
    id: "chat",
    contract: "sidebar",
    component: Probe,
    props: { label: "chat-panel" },
    tab: { icon: "💬", labelKey: "chat.tab" },
  });
  registry.contribute({
    id: "gameSettings",
    contract: "sidebar",
    component: Probe,
    props: { label: "settings-panel" },
    tab: { icon: "⚙️", labelKey: "gameSettings.tab", gmOnly: true },
  });
  registry.contribute({
    id: "untabbed",
    contract: "sidebar",
    component: Probe,
    props: { label: "untabbed-panel" },
  });
  return registry;
}

test("renders one rail button per contribution with icon + aria-label from t(labelKey)", () => {
  const registry = registryWithTabs();
  render(TabbedSurface, {
    props: { contract: "sidebar" },
    context: setAppContextForTest({ contributions: registry, role: "gm", t }),
  });

  const chatBtn = screen.getByTestId("tab-chat");
  expect(chatBtn.textContent).toBe("💬");
  expect(chatBtn.getAttribute("aria-label")).toBe("chat.tab");
});

test("gmOnly tab hidden for role player, shown for gm", () => {
  const registry = registryWithTabs();

  const { unmount } = render(TabbedSurface, {
    props: { contract: "sidebar" },
    context: setAppContextForTest({ contributions: registry, role: "player", t }),
  });
  expect(screen.queryByTestId("tab-gameSettings")).toBeNull();
  unmount();

  render(TabbedSurface, {
    props: { contract: "sidebar" },
    context: setAppContextForTest({ contributions: registry, role: "gm", t }),
  });
  expect(screen.getByTestId("tab-gameSettings")).toBeTruthy();
});

test("fallback metadata used when tab is absent: icon = first char of id, label = id", () => {
  const registry = registryWithTabs();
  render(TabbedSurface, {
    props: { contract: "sidebar" },
    context: setAppContextForTest({ contributions: registry, role: "gm", t }),
  });

  const untabbedBtn = screen.getByTestId("tab-untabbed");
  expect(untabbedBtn.textContent).toBe("u");
  expect(untabbedBtn.getAttribute("aria-label")).toBe("untabbed");
});

test("all tab panels are mounted; only the active one is visible via the hidden attribute", () => {
  const registry = registryWithTabs();
  render(TabbedSurface, {
    props: { contract: "sidebar" },
    context: setAppContextForTest({ contributions: registry, role: "gm", t }),
  });

  const chatPanel = screen.getByTestId("panel-chat") as HTMLElement;
  const settingsPanel = screen.getByTestId("panel-gameSettings") as HTMLElement;
  const untabbedPanel = screen.getByTestId("panel-untabbed") as HTMLElement;

  // First visible item (chat) is active by default.
  expect(chatPanel.hasAttribute("hidden")).toBe(false);
  expect(settingsPanel.hasAttribute("hidden")).toBe(true);
  expect(untabbedPanel.hasAttribute("hidden")).toBe(true);

  // Every panel's component is mounted regardless of visibility.
  expect(screen.getByText("chat-panel")).toBeTruthy();
  expect(screen.getByText("settings-panel")).toBeTruthy();
  expect(screen.getByText("untabbed-panel")).toBeTruthy();
});

test("clicking a tab calls onTabChange with the id and switches visibility", async () => {
  const registry = registryWithTabs();
  const onTabChange = (id: string) => {
    seen.push(id);
  };
  const seen: string[] = [];
  render(TabbedSurface, {
    props: { contract: "sidebar", onTabChange },
    context: setAppContextForTest({ contributions: registry, role: "gm", t }),
  });

  await fireEvent.click(screen.getByTestId("tab-untabbed"));
  expect(seen).toEqual(["untabbed"]);
});

test("activeId prop selects the tab; an activeId not in the visible set falls back to first", () => {
  const registry = registryWithTabs();
  render(TabbedSurface, {
    props: { contract: "sidebar", activeId: "untabbed" },
    context: setAppContextForTest({ contributions: registry, role: "gm", t }),
  });

  expect((screen.getByTestId("panel-untabbed") as HTMLElement).hasAttribute("hidden")).toBe(false);
  expect((screen.getByTestId("panel-chat") as HTMLElement).hasAttribute("hidden")).toBe(true);
});

test("an activeId that names a gmOnly tab hidden from the current role falls back to first visible", () => {
  const registry = registryWithTabs();
  render(TabbedSurface, {
    props: { contract: "sidebar", activeId: "gameSettings" },
    context: setAppContextForTest({ contributions: registry, role: "player", t }),
  });

  // gameSettings is filtered out for player; falls back to first visible (chat).
  expect((screen.getByTestId("panel-chat") as HTMLElement).hasAttribute("hidden")).toBe(false);
});

test("collapse toggle removes the content area; clicking a tab while collapsed re-expands", async () => {
  const registry = registryWithTabs();
  const seen: string[] = [];
  render(TabbedSurface, {
    props: { contract: "sidebar", onTabChange: (id: string) => seen.push(id) },
    context: setAppContextForTest({ contributions: registry, role: "gm", t }),
  });

  expect(screen.getByTestId("panel-chat")).toBeTruthy();

  await fireEvent.click(screen.getByLabelText("sidebar.collapse"));
  expect(screen.queryByTestId("panel-chat")).toBeNull();

  // Clicking a rail tab while collapsed re-expands the content area and still
  // reports the click via onTabChange (activeId selection remains parent-owned).
  await fireEvent.click(screen.getByTestId("tab-untabbed"));
  expect(screen.getByTestId("panel-untabbed")).toBeTruthy();
  expect(seen).toEqual(["untabbed"]);
});
