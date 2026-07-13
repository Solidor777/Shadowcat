import { test, expect, afterEach } from "vitest";
import { render, screen, cleanup } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { PanelsBridge } from "@shadowcat/ui-kit";
import { ContributionRegistry, PANEL_CONTRACT, silentLogger } from "@shadowcat/core";
import DockChipsContribution from "./DockChipsContribution.svelte";
import { PanelsController } from "./controller.svelte";
import { locate } from "./layout/tree";

afterEach(() => cleanup());

function registry(): ContributionRegistry {
  const r = new ContributionRegistry();
  r.contribute({
    id: "a:panel",
    contract: PANEL_CONTRACT,
    component: {},
    panel: { icon: "a", labelKey: "a.tab", defaultPlacement: { kind: "minimized" } },
  });
  r.contribute({
    id: "b:panel",
    contract: PANEL_CONTRACT,
    component: {},
    panel: { icon: "b", labelKey: "b.tab", defaultPlacement: { kind: "docked", zone: "right" } },
  });
  return r;
}

// `panels:chips` (rendered via the statusbar's `panel-dock` Surface) must
// reflect layout changes made to the shared controller AFTER this
// contribution itself mounted, not just a snapshot taken at mount time.
test("DockChipsContribution reflects a layout change made through the shared bridge after mount", async () => {
  const contributions = registry();
  const bridge = new PanelsBridge(silentLogger);
  const ctrl = new PanelsController({
    contributions,
    role: "gm",
    getPanelLayout: () => null,
    setPanelLayout: () => {},
    bridge,
    logger: silentLogger,
  });

  const context = setAppContextForTest({ contributions, panels: bridge });
  render(DockChipsContribution, { context });

  // "a:panel" starts minimized (its own defaultPlacement); "b:panel" starts docked.
  expect(screen.getByTestId("chip-a:panel")).toBeTruthy();
  expect(screen.queryByTestId("chip-b:panel")).toBeNull();

  // Mutate the same controller instance after mount — not through this
  // component, mirroring an engine gesture or another surface's call.
  ctrl.dispatch({ op: "minimize", id: "b:panel" });
  await Promise.resolve();

  expect(screen.getByTestId("chip-a:panel")).toBeTruthy();
  expect(screen.getByTestId("chip-b:panel")).toBeTruthy();
});

test("clicking a chip restores the panel through the bridge", async () => {
  const contributions = registry();
  const bridge = new PanelsBridge(silentLogger);
  const ctrl = new PanelsController({
    contributions,
    role: "gm",
    getPanelLayout: () => null,
    setPanelLayout: () => {},
    bridge,
    logger: silentLogger,
  });

  const context = setAppContextForTest({ contributions, panels: bridge });
  render(DockChipsContribution, { context });

  const chip = screen.getByTestId("chip-a:panel");
  chip.click();
  await Promise.resolve();

  expect(screen.queryByTestId("chip-a:panel")).toBeNull();
  expect(locate(ctrl.layout, "a:panel").where).toBe("docked");
});
