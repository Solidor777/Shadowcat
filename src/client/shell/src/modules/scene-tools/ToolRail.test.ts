import { render, screen, fireEvent } from "@testing-library/svelte";
import { test, expect } from "vitest";
import type { SceneTool } from "@shadowcat/render";
import { SceneInteractionBridge } from "@shadowcat/ui-kit";
import { fakeSceneHost } from "@shadowcat/ui-kit/test";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import ToolRail from "./ToolRail.svelte";

/** A bridge with an attached host that records every setActiveTool call. */
function captureScene(): { scene: SceneInteractionBridge; tools: (SceneTool | null)[] } {
  const tools: (SceneTool | null)[] = [];
  const scene = new SceneInteractionBridge();
  scene.attach(fakeSceneHost({ setActiveTool: (t) => tools.push(t) }));
  return { scene, tools };
}

test("a GM sees tool buttons; selecting toggles the active tool on the scene", async () => {
  const { scene, tools } = captureScene();
  render(ToolRail, { context: setAppContextForTest({ role: "gm", scene }) });

  const select = screen.getByTestId("tool-select");
  await fireEvent.click(select);
  expect(tools.at(-1)).not.toBeNull(); // a tool was activated
  expect(select.getAttribute("aria-pressed")).toBe("true");

  await fireEvent.click(select);
  expect(tools.at(-1)).toBeNull(); // toggled off
  expect(select.getAttribute("aria-pressed")).toBe("false");
});

test("selecting a different tool switches the active tool", async () => {
  const { scene, tools } = captureScene();
  render(ToolRail, { context: setAppContextForTest({ role: "gm", scene }) });
  await fireEvent.click(screen.getByTestId("tool-select"));
  await fireEvent.click(screen.getByTestId("tool-place"));
  expect(tools.at(-1)).not.toBeNull(); // place tool now active (select replaced)
  expect(screen.getByTestId("tool-select").getAttribute("aria-pressed")).toBe("false");
  expect(screen.getByTestId("tool-place").getAttribute("aria-pressed")).toBe("true");
});

test("the draw and template tools activate and reveal their controls", async () => {
  const { scene, tools } = captureScene();
  render(ToolRail, { context: setAppContextForTest({ role: "gm", scene }) });
  await fireEvent.click(screen.getByTestId("tool-draw"));
  expect(tools.at(-1)).not.toBeNull();
  expect(screen.getByTestId("draw-mode")).toBeTruthy(); // draw controls shown
  await fireEvent.click(screen.getByTestId("tool-template"));
  expect(screen.getByTestId("template-mode")).toBeTruthy();
  expect(screen.queryByTestId("draw-mode")).toBeNull(); // switched away from draw
});

test("the measure and ping tools are available and activate", async () => {
  const { scene, tools } = captureScene();
  render(ToolRail, { context: setAppContextForTest({ role: "gm", scene }) });
  await fireEvent.click(screen.getByTestId("tool-measure"));
  expect(tools.at(-1)).not.toBeNull();
  await fireEvent.click(screen.getByTestId("tool-ping"));
  expect(tools.at(-1)).not.toBeNull();
  expect(screen.getByTestId("tool-ping").getAttribute("aria-pressed")).toBe("true");
});

test("a non-GM sees no tool buttons", () => {
  render(ToolRail, { context: setAppContextForTest({ role: "player" }) });
  expect(screen.queryByTestId("tool-select")).toBeNull();
  expect(screen.queryByTestId("tool-place")).toBeNull();
  expect(screen.queryByTestId("tool-draw")).toBeNull();
});
