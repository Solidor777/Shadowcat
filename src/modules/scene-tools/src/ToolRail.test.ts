import { render, screen, fireEvent } from "@testing-library/svelte";
import { test, expect } from "vitest";
import type { SceneTool } from "@shadowcat/render";
import { SceneInteractionBridge } from "@shadowcat/ui-kit";
import { fakeSceneHost } from "@shadowcat/ui-kit/test";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore, buildSceneDoc, buildTokenDoc, type WireOperation } from "@shadowcat/core";
import { TokenSelection, SpeakAsToken } from "@shadowcat/ui-kit";
import ToolRail from "./ToolRail.svelte";
import toolRailSource from "./ToolRail.svelte?raw";

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

// Regression: AppContext.moveRequest exists (setAppContextForTest defaults it, and
// commitRoute's own unit tests in `measure-tool.test` prove the commit logic works when
// wired) but ToolRail's `new ToolController({...})` call omitted the field, so the
// double-click route-commit was permanently unreachable through the real UI — silently
// falling back to `commitRoute`'s "moveRequest absent" no-op every time, regardless of
// connection state. Drives the tool instance ToolRail itself hands to the scene bridge,
// exactly as the render engine would via real pointer events.
test("the measure tool's double-click route-commit reaches AppContext.moveRequest (ToolRail must wire it into the controller)", async () => {
  const docs = sceneStore();
  docs.applyCommand({
    seq: 2, world_id: "w1", author: "a", ts: 0,
    ops: [{
      op: "create",
      doc: buildTokenDoc("w1", "s1", {
        x: 0, y: 0, w: 100, h: 100, rotation: 0,
        visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null,
      }, "tok1"),
    }],
  });
  const sel = new TokenSelection();
  sel.set(["tok1"]);

  const moves: Array<{ tokenId: string; path: [number, number][] }> = [];
  const { scene, tools } = captureScene();
  render(ToolRail, {
    context: setAppContextForTest({
      role: "gm",
      scene,
      documents: docs,
      tokenSelection: sel,
      pathfind: async () => ({ path: [[0, 0], [100, 0]], cost: 1, arrested: false, truncated: false }),
      moveRequest: async (_s, tokenId, path) => {
        moves.push({ tokenId, path });
        return {
          requestId: "r1", tokenId, mover: "u1", scene: "s1", startServerMs: 0,
          durationMs: 300, stop: path.at(-1)!, samples: [], moverVision: null, cost: 1, truncated: false,
        };
      },
    }),
  });
  await fireEvent.click(screen.getByTestId("tool-measure"));
  const tool = tools.at(-1)!;

  // Double-click commit gesture (mirrors `measure-tool.test`'s own commit tests).
  const ev = {} as PointerEvent;
  tool.onPointerDown({ x: 100, y: 100 }, ev);
  tool.onPointerUp({ x: 100, y: 100 }, ev);
  tool.onPointerDown({ x: 100, y: 100 }, ev);
  tool.onPointerUp({ x: 100, y: 100 }, ev);
  await new Promise((r) => setTimeout(r, 0)); // drain the pathfind/moveRequest microtasks

  expect(moves.length).toBe(1);
  expect(moves[0].tokenId).toBe("tok1");
});

/** Every authoring tool: creates or edits scene content, GM-only. */
const AUTHORING_TOOLS = ["place", "draw", "template", "wall", "region"] as const;

test("a non-GM sees exactly the player tools (select/measure/ping) and NO authoring tool", () => {
  render(ToolRail, { context: setAppContextForTest({ role: "player" }) });
  expect(screen.getByTestId("tool-select")).toBeTruthy();
  expect(screen.getByTestId("tool-measure")).toBeTruthy();
  expect(screen.getByTestId("tool-ping")).toBeTruthy();
  // Negative assertion per authoring tool: a length/count check passes even when the
  // WRONG set of three is rendered, so each absent tool is named individually.
  for (const id of AUTHORING_TOOLS) {
    expect(screen.queryByTestId(`tool-${id}`)).toBeNull();
  }
});

test("a GM still sees the full rail (every player tool AND every authoring tool)", () => {
  render(ToolRail, { context: setAppContextForTest({ role: "gm" }) });
  for (const id of ["select", "measure", "ping", ...AUTHORING_TOOLS]) {
    expect(screen.getByTestId(`tool-${id}`)).toBeTruthy();
  }
});

// The controller is built for EVERY user; only the authoring entries are role-conditional.
// A rail whose ToolController construction sits inside a single `{#if isGm}` leaves a non-GM
// with no active tool at all, and every canvas drag then falls through to camera pan.
test("a non-GM's ToolController is constructed and activates the select tool on the scene bridge", async () => {
  const { scene, tools } = captureScene();
  render(ToolRail, { context: setAppContextForTest({ role: "player", scene }) });
  await fireEvent.click(screen.getByTestId("tool-select"));
  expect(tools.at(-1)).not.toBeNull();
  expect(screen.getByTestId("tool-select").getAttribute("aria-pressed")).toBe("true");
});

// Pins the write path a non-GM's drag reaches: since a player's move is server-executed
// (`execute_move`, gated by wall + visibility-mask + arrest regions), the client must never
// write a token position directly — it requests one via pathfind + moveRequest instead, and
// the rendered position only advances once the resulting MoveStream arrives.
test("a non-GM's select drag issues a moveRequest and writes no document update", async () => {
  const docs = sceneStore();
  docs.applyCommand({
    seq: 2, world_id: "w1", author: "a", ts: 0,
    ops: [{
      op: "create",
      doc: buildTokenDoc("w1", "s1", {
        x: 0, y: 0, w: 100, h: 100, rotation: 0,
        visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null,
      }, "tok1"),
    }],
  });
  const dispatched: WireOperation[][] = [];
  const moves: Array<{ tokenId: string; path: [number, number][] }> = [];
  const { scene, tools } = captureScene();
  render(ToolRail, {
    context: setAppContextForTest({
      role: "player",
      scene,
      documents: docs,
      tokenSelection: new TokenSelection(),
      dispatchIntent: (ops) => dispatched.push(ops),
      pathfind: async (_s, start, waypoints) => ({ path: [start, waypoints.at(-1)!], cost: 1, arrested: false, truncated: false }),
      moveRequest: async (s, tokenId, path) => {
        moves.push({ tokenId, path });
        return {
          requestId: "r1", tokenId, mover: "u1", scene: s, startServerMs: 0,
          durationMs: 300, stop: path.at(-1)!, samples: [], moverVision: null, cost: 1, truncated: false,
        };
      },
    }),
  });
  await fireEvent.click(screen.getByTestId("tool-select"));
  const tool = tools.at(-1)!;
  const ev = { shiftKey: false } as PointerEvent;
  tool.onPointerDown({ x: 0, y: 0 }, ev);
  tool.onPointerMove({ x: 100, y: 0 }, ev);
  tool.onPointerUp({ x: 100, y: 0 }, ev);
  await new Promise((r) => setTimeout(r, 0)); // drain the pathfind/moveRequest microtasks

  expect(dispatched).toEqual([]);
  expect(moves).toEqual([{ tokenId: "tok1", path: [[0, 0], [100, 0]] }]);
});

/** A DocumentStore seeded with one scene doc carrying `system`. */
function sceneStore(system: Record<string, unknown> = {}): DocumentStore {
  const docs = new DocumentStore();
  docs.applyCommand({
    seq: 1, world_id: "w1", author: "a", ts: 0,
    ops: [{ op: "create", doc: buildSceneDoc("w1", system, "s1") }],
  });
  return docs;
}

test("the snap toggle reflects the resolved snapToGrid (grid-stepped default: pressed) and dispatches an update on click", async () => {
  const dispatched: WireOperation[][] = [];
  render(ToolRail, {
    context: setAppContextForTest({
      role: "gm",
      documents: sceneStore(),
      dispatchIntent: (ops) => dispatched.push(ops),
    }),
  });
  const toggle = screen.getByTestId("snap-toggle");
  expect(toggle.getAttribute("aria-pressed")).toBe("true"); // grid-stepped default
  await fireEvent.click(toggle);
  expect(dispatched.at(-1)).toEqual([
    { op: "update", doc_id: "s1", changes: [{ path: "/engine/snapToGrid", old: null, new: false }] },
  ]);
});

test("the snap toggle sends the ACTUAL stored value as `old`, not null, when snapToGrid was already explicitly stored (regression: repeated toggles must not hit a stale optimistic-concurrency conflict)", async () => {
  const dispatched: WireOperation[][] = [];
  render(ToolRail, {
    context: setAppContextForTest({
      role: "gm",
      documents: sceneStore({ snapToGrid: true }),
      dispatchIntent: (ops) => dispatched.push(ops),
    }),
  });
  const toggle = screen.getByTestId("snap-toggle");
  expect(toggle.getAttribute("aria-pressed")).toBe("true");
  await fireEvent.click(toggle);
  expect(dispatched.at(-1)).toEqual([
    { op: "update", doc_id: "s1", changes: [{ path: "/engine/snapToGrid", old: true, new: false }] },
  ]);
});

test("the snap toggle reflects a continuous scene's false default", () => {
  render(ToolRail, {
    context: setAppContextForTest({
      role: "gm",
      documents: sceneStore({ vision: { movementModel: "continuous" } }),
    }),
  });
  expect(screen.getByTestId("snap-toggle").getAttribute("aria-pressed")).toBe("false");
});

test("no active scene: the snap toggle does not render", () => {
  render(ToolRail, { context: setAppContextForTest({ role: "gm", documents: new DocumentStore() }) });
  expect(screen.queryByTestId("snap-toggle")).toBeNull();
});

test("a non-GM does not see the snap toggle even with an active scene", () => {
  render(ToolRail, { context: setAppContextForTest({ role: "player", documents: sceneStore() }) });
  expect(screen.queryByTestId("snap-toggle")).toBeNull();
});

test("the tool rail renders as a non-compact side rail under jsdom (expanded default)", () => {
  const { scene } = captureScene();
  const { container } = render(ToolRail, { context: setAppContextForTest({ role: "gm", scene }) });
  const rail = container.querySelector(".tool-rail");
  expect(rail).toBeTruthy();
  expect(rail?.classList.contains("compact")).toBe(false);
});

test("a player who owns the single selected token sees and can use the speak-as-token button", async () => {
  const { scene } = captureScene();
  const documents = new DocumentStore();
  const sceneDoc = buildSceneDoc("w1", {}, "S1");
  documents.applyCommand({ seq: 1, world_id: "w1", author: "gm", ts: 0, ops: [{ op: "create" as const, doc: sceneDoc }] });
  const token = {
    ...buildTokenDoc(
      "w1", sceneDoc.id,
      { x: 0, y: 0, w: 1, h: 1, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null },
      "tok1",
    ),
    owner: "u-self",
  };
  documents.applyCommand({ seq: 2, world_id: "w1", author: "u-self", ts: 0, ops: [{ op: "create" as const, doc: token }] });
  const tokenSelection = new TokenSelection();
  tokenSelection.set(["tok1"]);
  const speakAsToken = new SpeakAsToken();
  render(ToolRail, { context: setAppContextForTest({ role: "player", selfId: "u-self", scene, documents, tokenSelection, speakAsToken }) });
  const button = screen.getByTestId("speak-as-token");
  await fireEvent.click(button);
  expect(speakAsToken.tokenId).toBe("tok1");
});

test("a non-owner player does not see the speak-as-token button", () => {
  const { scene } = captureScene();
  const documents = new DocumentStore();
  const sceneDoc = buildSceneDoc("w1", {}, "S1");
  documents.applyCommand({ seq: 1, world_id: "w1", author: "gm", ts: 0, ops: [{ op: "create" as const, doc: sceneDoc }] });
  const token = {
    ...buildTokenDoc(
      "w1", sceneDoc.id,
      { x: 0, y: 0, w: 1, h: 1, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null },
      "tok1",
    ),
    owner: "someone-else",
  };
  documents.applyCommand({ seq: 2, world_id: "w1", author: "gm", ts: 0, ops: [{ op: "create" as const, doc: token }] });
  const tokenSelection = new TokenSelection();
  tokenSelection.set(["tok1"]);
  render(ToolRail, { context: setAppContextForTest({ role: "player", selfId: "u-self", scene, documents, tokenSelection }) });
  expect(screen.queryByTestId("speak-as-token")).toBeNull();
});

test("no button renders when zero tokens are selected", () => {
  const { scene } = captureScene();
  render(ToolRail, { context: setAppContextForTest({ role: "gm", scene }) });
  expect(screen.queryByTestId("speak-as-token")).toBeNull();
});

test("no button renders when more than one token is selected, even for a GM", () => {
  const { scene } = captureScene();
  const documents = new DocumentStore();
  const sceneDoc = buildSceneDoc("w1", {}, "S1");
  documents.applyCommand({ seq: 1, world_id: "w1", author: "gm", ts: 0, ops: [{ op: "create" as const, doc: sceneDoc }] });
  const engine = { x: 0, y: 0, w: 1, h: 1, rotation: 0, visual: { kind: "image" as const, asset: "a" }, actor_id: null, overrides: null, face: null };
  const tok1 = buildTokenDoc("w1", sceneDoc.id, engine, "tok1");
  const tok2 = buildTokenDoc("w1", sceneDoc.id, engine, "tok2");
  documents.applyCommand({ seq: 2, world_id: "w1", author: "gm", ts: 0, ops: [{ op: "create" as const, doc: tok1 }, { op: "create" as const, doc: tok2 }] });
  const tokenSelection = new TokenSelection();
  tokenSelection.set(["tok1", "tok2"]);
  render(ToolRail, { context: setAppContextForTest({ role: "gm", scene, documents, tokenSelection }) });
  expect(screen.queryByTestId("speak-as-token")).toBeNull();
});

test("select/input controls get a touch-target-floor coarse-pointer min-height", () => {
  // jsdom doesn't evaluate @media (pointer: coarse), so assert the rule's
  // presence directly in the component's source styles instead (mirrors the
  // "select/input controls get a 44px coarse-pointer min-height" convention used elsewhere).
  const controlsRuleMatch = toolRailSource.match(/\.controls select,\s*\.controls input\s*\{([^}]*@media[^}]*\{[^}]*\}[^}]*)\}/);
  expect(controlsRuleMatch).toBeTruthy();
  expect(controlsRuleMatch?.[1]).toMatch(/@media \(pointer: coarse\)\s*\{\s*min-height:\s*var\(--input-height-coarse\);\s*\}/);
});

test("select/input controls fit the rail's content box instead of overflowing it", () => {
  // The rail's containing cell is sized to exactly the touch-target floor (see
  // `Layout.test`'s toolrail-column test), so every child — buttons already carried
  // `min-width: 44px` — must also cap its own box at the container width or it clips.
  const controlsRuleMatch = toolRailSource.match(/\.controls select,\s*\.controls input\s*\{([^}]*)\}/);
  expect(controlsRuleMatch).toBeTruthy();
  expect(controlsRuleMatch?.[1]).toMatch(/min-width:\s*0;/);
  expect(controlsRuleMatch?.[1]).toMatch(/max-width:\s*100%;/);
  expect(controlsRuleMatch?.[1]).toMatch(/box-sizing:\s*border-box;/);
});
