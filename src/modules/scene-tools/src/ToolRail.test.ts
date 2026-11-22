import { render, screen, fireEvent, waitFor } from "@testing-library/svelte";
import { test, expect } from "vitest";
import type { SceneTool } from "@shadowcat/render";
import { SceneInteractionBridge, type AppContext } from "@shadowcat/ui-kit";
import { fakeSceneHost } from "@shadowcat/ui-kit/test";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore, buildSceneDoc, buildTokenDoc, buildLightDoc, buildSceneEntityDoc, buildRegionDoc, ContributionRegistry, SCENE_TOOL_CONTRACT, type WireOperation } from "@shadowcat/core";
import { TokenSelection, SpeakAsToken, SnapModePreference, ShareRulerPreference } from "@shadowcat/ui-kit";
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

test("the template tool's angle/width inputs appear only for their respective shape modes", async () => {
  const { scene } = captureScene();
  render(ToolRail, { context: setAppContextForTest({ role: "gm", scene }) });
  await fireEvent.click(screen.getByTestId("tool-template"));

  // Default mode is "circle": neither input is shown.
  expect(screen.queryByTestId("template-angle")).toBeNull();
  expect(screen.queryByTestId("template-width")).toBeNull();

  await fireEvent.change(screen.getByTestId("template-mode"), { target: { value: "cone" } });
  expect(screen.getByTestId("template-angle")).toBeTruthy();
  expect(screen.queryByTestId("template-width")).toBeNull();

  await fireEvent.change(screen.getByTestId("template-mode"), { target: { value: "line" } });
  expect(screen.queryByTestId("template-angle")).toBeNull();
  expect(screen.getByTestId("template-width")).toBeTruthy();

  await fireEvent.change(screen.getByTestId("template-mode"), { target: { value: "rect" } });
  expect(screen.queryByTestId("template-angle")).toBeNull();
  expect(screen.getByTestId("template-width")).toBeTruthy();
});

test("selecting the ambient_light region behavior reveals the mode/level inputs; switching away hides them", async () => {
  const { scene } = captureScene();
  render(ToolRail, { context: setAppContextForTest({ role: "gm", scene }) });
  await fireEvent.click(screen.getByTestId("tool-region"));
  expect(screen.queryByTestId("region-ambient-mode")).toBeNull();
  expect(screen.queryByTestId("region-ambient-level")).toBeNull();

  await fireEvent.change(screen.getByTestId("region-behavior"), { target: { value: "ambient_light" } });
  expect(screen.getByTestId("region-ambient-mode")).toBeTruthy();
  expect(screen.getByTestId("region-ambient-level")).toBeTruthy();

  await fireEvent.change(screen.getByTestId("region-behavior"), { target: { value: "terrain" } });
  expect(screen.queryByTestId("region-ambient-mode")).toBeNull();
  expect(screen.queryByTestId("region-ambient-level")).toBeNull();
});

test("the per-tag cost table is shown only for the terrain behavior", async () => {
  const { scene } = captureScene();
  render(ToolRail, { context: setAppContextForTest({ role: "gm", scene }) });
  await fireEvent.click(screen.getByTestId("tool-region"));
  // Default behavior is "terrain": the add button is present.
  expect(screen.getByTestId("region-cost-by-tag-add")).toBeTruthy();

  await fireEvent.change(screen.getByTestId("region-behavior"), { target: { value: "impassable" } });
  expect(screen.queryByTestId("region-cost-by-tag-add")).toBeNull();

  await fireEvent.change(screen.getByTestId("region-behavior"), { target: { value: "terrain" } });
  expect(screen.getByTestId("region-cost-by-tag-add")).toBeTruthy();
});

test("adding, editing and removing per-tag cost rows through the rail persists the surviving rows", async () => {
  const { scene, tools } = captureScene();
  const dispatched: WireOperation[][] = [];
  render(ToolRail, {
    context: setAppContextForTest({
      role: "gm",
      scene,
      documents: sceneStore(),
      dispatchIntent: (ops) => dispatched.push(ops),
    }),
  });
  await fireEvent.click(screen.getByTestId("tool-region"));

  // Add two rows and fill each through its own inputs.
  await fireEvent.click(screen.getByTestId("region-cost-by-tag-add"));
  await fireEvent.click(screen.getByTestId("region-cost-by-tag-add"));
  await waitFor(() => expect(screen.getAllByTestId("region-cost-by-tag-row")).toHaveLength(2));
  const tagInputs = screen.getAllByTestId("region-cost-by-tag-tag") as HTMLInputElement[];
  const costInputs = screen.getAllByTestId("region-cost-by-tag-cost") as HTMLInputElement[];
  await fireEvent.input(tagInputs[0], { target: { value: "flying" } });
  await fireEvent.input(costInputs[0], { target: { value: "1" } });
  await fireEvent.input(tagInputs[1], { target: { value: "swimming" } });
  await fireEvent.input(costInputs[1], { target: { value: "3" } });
  expect((tagInputs[0] as HTMLInputElement).value).toBe("flying");
  expect((costInputs[1] as HTMLInputElement).value).toBe("3");

  // Remove the first row; only "swimming" survives.
  const removeButtons = screen.getAllByTestId("region-cost-by-tag-remove");
  await fireEvent.click(removeButtons[0]);
  await waitFor(() => expect(screen.getAllByTestId("region-cost-by-tag-row")).toHaveLength(1));
  expect((screen.getByTestId("region-cost-by-tag-tag") as HTMLInputElement).value).toBe("swimming");

  // Drag to persist: the created region's cost_by_tag carries only the surviving row.
  const tool = tools.at(-1)!;
  const ev = {} as PointerEvent;
  tool.onPointerDown({ x: 0, y: 0 }, ev);
  tool.onPointerUp({ x: 100, y: 100 }, ev);
  expect(dispatched).toHaveLength(1);
  const op = dispatched[0][0];
  expect(op.op).toBe("create");
  if (op.op === "create") {
    expect(op.doc.engine).toMatchObject({ cost_by_tag: { swimming: 3 } });
  }
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
        visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null, elevation: null, lock_rotation: false,
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
      pathfind: async () => ({ path: [[0, 0], [100, 0]], cost: 1, arrested: false, truncated: false, budgetCells: null }),
      moveRequest: async (_s, tokenId, path) => {
        moves.push({ tokenId, path });
        return {
          requestId: "r1", tokenId, mover: "u1", scene: "s1", startServerMs: 0,
          durationMs: 300, stop: path.at(-1)!, samples: [], moverVision: null, moverLight: null, cost: 1, truncated: false,
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

// Regression, same class as the `moveRequest` omission above: `AppContext.t` exists and
// `requestRoute`'s label logic is proven under a unit-level `ToolContext`, but ToolRail's
// `new ToolController({...})` literal is the ONE place the host's seams become the tools',
// and a member it leaves out degrades silently. Drives the real rail with a translator that
// marks every call, so the label can only carry the marker if the rail wired `t` through.
test("the measure tool's Warn-overage label reaches AppContext.t (ToolRail must wire it into the controller)", async () => {
  const docs = new DocumentStore();
  docs.applyCommand({
    seq: 1, world_id: "w1", author: "a", ts: 0,
    ops: [
      { op: "create", doc: buildSceneDoc("w1", { grid: { kind: "square", size: 100, distance: { perCell: 5, unit: "ft" } } }, "s1") },
      {
        op: "create",
        doc: buildTokenDoc("w1", "s1", {
          x: 50, y: 50, w: 100, h: 100, rotation: 0,
          visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null, elevation: null, lock_rotation: false,
        }, "tok1"),
      },
    ],
  });
  const sel = new TokenSelection();
  sel.set(["tok1"]);
  const labels: string[] = [];
  const tools: (SceneTool | null)[] = [];
  const scene = new SceneInteractionBridge();
  scene.attach(fakeSceneHost({
    setActiveTool: (t) => tools.push(t),
    snap: (p) => p,
    drawMeasure: (_f, _t, label) => labels.push(label),
  }));
  const combat = {
    activeFor: () => ({ id: "combat-1", engine: { movement: { resource: "movement", interpretation: "per_cell", enforcement: "warn" } } }),
  } as unknown as AppContext["combat"];
  render(ToolRail, {
    context: setAppContextForTest({
      role: "gm",
      scene,
      documents: docs,
      tokenSelection: sel,
      combat,
      t: (key, params) => `<${key}${params ? ":" + JSON.stringify(params) : ""}>`,
      pathfind: async () => ({ path: [[50, 50], [150, 50]], cost: 7, arrested: false, truncated: false, budgetCells: 5 }),
    }),
  });
  await fireEvent.click(screen.getByTestId("tool-measure"));
  const tool = tools.at(-1)!;
  const ev = {} as PointerEvent;
  tool.onPointerDown({ x: 50, y: 50 }, ev);
  tool.onPointerMove({ x: 150, y: 50 }, ev);
  await new Promise((r) => setTimeout(r, 0)); // drain the pathfind microtasks

  expect(labels.at(-1)).toBe('35 ft · <tools.overBudget:{"over":"10 ft"}>');
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
        visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null, elevation: null, lock_rotation: false,
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
      pathfind: async (_s, start, waypoints) => ({ path: [start, waypoints.at(-1)!], cost: 1, arrested: false, truncated: false, budgetCells: null }),
      moveRequest: async (s, tokenId, path) => {
        moves.push({ tokenId, path });
        return {
          requestId: "r1", tokenId, mover: "u1", scene: s, startServerMs: 0,
          durationMs: 300, stop: path.at(-1)!, samples: [], moverVision: null, moverLight: null, cost: 1, truncated: false,
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

test("the snap-mode toggle flips the session preference and reflects it (center default)", async () => {
  const snapModePreference = new SnapModePreference();
  render(ToolRail, {
    context: setAppContextForTest({ role: "gm", documents: sceneStore(), snapModePreference }),
  });
  const toggle = screen.getByTestId("snap-mode-toggle");
  expect(toggle.getAttribute("aria-pressed")).toBe("false"); // default: center mode
  await fireEvent.click(toggle);
  expect(snapModePreference.mode).toBe("vertex");
  expect(toggle.getAttribute("aria-pressed")).toBe("true");
  await fireEvent.click(toggle);
  expect(snapModePreference.mode).toBe("edge");
  expect(toggle.getAttribute("aria-pressed")).toBe("true");
  await fireEvent.click(toggle);
  expect(snapModePreference.mode).toBe("center");
  expect(toggle.getAttribute("aria-pressed")).toBe("false");
});

test("the snap-mode toggle dispatches no document write (client-local session state)", async () => {
  const dispatched: WireOperation[][] = [];
  render(ToolRail, {
    context: setAppContextForTest({
      role: "gm",
      documents: sceneStore(),
      dispatchIntent: (ops) => dispatched.push(ops),
    }),
  });
  await fireEvent.click(screen.getByTestId("snap-mode-toggle"));
  expect(dispatched).toEqual([]);
});

test("the share-ruler toggle is offered only while the measure tool is active, and is off by default", async () => {
  const shareRuler = new ShareRulerPreference("u1");
  render(ToolRail, { context: setAppContextForTest({ role: "player", documents: sceneStore(), shareRuler }) });
  // Hidden until the measure tool is chosen: off by default and easy to miss unless looked for.
  expect(screen.queryByTestId("share-ruler-toggle")).toBeNull();
  await fireEvent.click(screen.getByTestId("tool-measure"));
  const toggle = screen.getByTestId("share-ruler-toggle");
  expect(toggle.getAttribute("aria-pressed")).toBe("false");
  // Accessible name present and identical to the tooltip (the harness `t` echoes the key).
  expect(toggle.getAttribute("aria-label")).toBeTruthy();
  expect(toggle.getAttribute("aria-label")).toBe(toggle.getAttribute("title"));
  expect(shareRuler.enabled).toBe(false);
  // Choosing another tool hides it again.
  await fireEvent.click(screen.getByTestId("tool-ping"));
  expect(screen.queryByTestId("share-ruler-toggle")).toBeNull();
});

test("the share-ruler toggle flips the local preference, reflects it, and dispatches no document write", async () => {
  const shareRuler = new ShareRulerPreference("u1");
  const dispatched: WireOperation[][] = [];
  render(ToolRail, {
    context: setAppContextForTest({
      role: "player",
      documents: sceneStore(),
      shareRuler,
      dispatchIntent: (ops) => dispatched.push(ops),
    }),
  });
  await fireEvent.click(screen.getByTestId("tool-measure"));
  const toggle = screen.getByTestId("share-ruler-toggle");
  await fireEvent.click(toggle);
  expect(shareRuler.enabled).toBe(true);
  expect(toggle.getAttribute("aria-pressed")).toBe("true");
  await fireEvent.click(toggle);
  expect(shareRuler.enabled).toBe(false);
  expect(toggle.getAttribute("aria-pressed")).toBe("false");
  expect(dispatched).toEqual([]);
});

test("no active scene: the snap-mode toggle does not render", () => {
  render(ToolRail, { context: setAppContextForTest({ role: "gm", documents: new DocumentStore() }) });
  expect(screen.queryByTestId("snap-mode-toggle")).toBeNull();
});

test("a non-GM does not see the snap-mode toggle even with an active scene", () => {
  render(ToolRail, { context: setAppContextForTest({ role: "player", documents: sceneStore() }) });
  expect(screen.queryByTestId("snap-mode-toggle")).toBeNull();
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
      { x: 0, y: 0, w: 1, h: 1, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null, elevation: null, lock_rotation: false },
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

/** Documents with scene `S1`, an `S2`, and tokens: two owned by `u-self` on S1 at (100,200) and
 * (300,400), one owned by someone else on S1, one owned by `u-self` on S2. */
function ownedTokenStore(): DocumentStore {
  const documents = new DocumentStore();
  documents.applyCommand({
    seq: 1, world_id: "w1", author: "gm", ts: 0,
    ops: [
      { op: "create" as const, doc: buildSceneDoc("w1", {}, "S1") },
      { op: "create" as const, doc: buildSceneDoc("w1", {}, "S2") },
    ],
  });
  const tok = (id: string, scene: string, x: number, y: number, owner: string) => ({
    ...buildTokenDoc(
      "w1", scene,
      { x, y, w: 1, h: 1, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null, elevation: null, lock_rotation: false },
      id,
    ),
    owner,
  });
  documents.applyCommand({
    seq: 2, world_id: "w1", author: "gm", ts: 0,
    ops: [
      { op: "create" as const, doc: tok("mine-a", "S1", 100, 200, "u-self") },
      { op: "create" as const, doc: tok("mine-b", "S1", 300, 400, "u-self") },
      { op: "create" as const, doc: tok("theirs", "S1", 9000, 9000, "someone-else") },
      { op: "create" as const, doc: tok("mine-elsewhere", "S2", 5000, 5000, "u-self") },
    ],
  });
  return documents;
}

test("pan-to-owned pans the camera to the average of the viewer's own tokens on the viewed scene", async () => {
  const pans: unknown[][] = [];
  const scene = new SceneInteractionBridge();
  scene.attach(fakeSceneHost({ panToPoint: (x, y) => pans.push([x, y]) }));
  render(ToolRail, {
    context: setAppContextForTest({ role: "player", selfId: "u-self", scene, documents: ownedTokenStore(), viewedSceneId: "S1" }),
  });
  const button = screen.getByTestId("pan-to-owned");
  expect(button.hasAttribute("disabled")).toBe(false);
  await fireEvent.click(button);
  // The average of the two S1 tokens this user owns; another user's token and the token on S2 are excluded.
  expect(pans).toEqual([[200, 300]]);
});

test("pan-to-owned is disabled, and pans nowhere, when the viewer owns no token on the viewed scene", async () => {
  const pans: unknown[][] = [];
  const scene = new SceneInteractionBridge();
  scene.attach(fakeSceneHost({ panToPoint: (x, y) => pans.push([x, y]) }));
  render(ToolRail, {
    context: setAppContextForTest({ role: "player", selfId: "u-other", scene, documents: ownedTokenStore(), viewedSceneId: "S1" }),
  });
  const button = screen.getByTestId("pan-to-owned");
  expect(button.hasAttribute("disabled")).toBe(true);
  await fireEvent.click(button);
  expect(pans).toEqual([]);
});

test("a non-owner player does not see the speak-as-token button", () => {
  const { scene } = captureScene();
  const documents = new DocumentStore();
  const sceneDoc = buildSceneDoc("w1", {}, "S1");
  documents.applyCommand({ seq: 1, world_id: "w1", author: "gm", ts: 0, ops: [{ op: "create" as const, doc: sceneDoc }] });
  const token = {
    ...buildTokenDoc(
      "w1", sceneDoc.id,
      { x: 0, y: 0, w: 1, h: 1, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null, elevation: null, lock_rotation: false },
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
  const engine = { x: 0, y: 0, w: 1, h: 1, rotation: 0, visual: { kind: "image" as const, asset: "a" }, actor_id: null, overrides: null, face: null, elevation: null, lock_rotation: false };
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

// --- Light/wall editors (the shared editing selection) ---

/** A store with one scene, one light at (200,200), one wall along y=700, one rect region at
 * (1000,1000)-(1100,1100), one rect drawing at (1300,1300)-(1400,1400), and one circle template
 * at (1600,1600) r=50 — every shape spatially disjoint so the select tool's light→wall→region→
 * drawing→template pick precedence never has to arbitrate between fixtures. */
function editorStore(): DocumentStore {
  const docs = sceneStore();
  docs.applyCommand({
    seq: 2, world_id: "w1", author: "a", ts: 0,
    ops: [
      {
        op: "create",
        doc: buildLightDoc("w1", "s1", {
          x: 200, y: 200,
          elevation: null,
          emission: { color: "#ffcc66", intensity: 0.8, brightRadius: 2, dimRadius: 6, falloff: null, enabled: true, rotation: 0, followTokenRotation: false },
        }, "light-1"),
      },
      {
        op: "create",
        doc: buildSceneEntityDoc("w1", "s1", "wall", {
          seg: { x1: 0, y1: 700, x2: 400, y2: 700 }, blocksSight: true, blocksMove: true, blocksLight: true,
        }),
      },
      {
        op: "create",
        doc: buildRegionDoc("w1", "s1", {
          shape: { kind: "rect", points: [1000, 1000, 1100, 1100] },
          behavior: "terrain", cost: 1, cost_by_tag: {}, enabled: true, triggers: [], elevation: null, ambient_override: null,
        }, "region-1"),
      },
      {
        op: "create",
        doc: buildSceneEntityDoc("w1", "s1", "drawing", {
          shape: { kind: "rect", points: [1300, 1300, 1400, 1400] },
          stroke: null, fill: null, elevation: null,
        }, "drawing-1"),
      },
      {
        op: "create",
        doc: buildSceneEntityDoc("w1", "s1", "template", {
          shape: { kind: "circle", x: 1600, y: 1600, size: 50, direction: 0 },
          color: "#3388ff", elevation: null,
        }, "template-1"),
      },
    ],
  });
  return docs;
}

test("the light tool is GM-only in the rail; a GM sees it, a player does not", () => {
  const { scene } = captureScene();
  render(ToolRail, { context: setAppContextForTest({ role: "gm", scene }) });
  expect(screen.getByTestId("tool-light")).toBeTruthy();

  const second = captureScene();
  render(ToolRail, { context: setAppContextForTest({ role: "player", scene: second.scene }) });
  expect(screen.queryAllByTestId("tool-light")).toHaveLength(1); // only the GM instance's
});

test("clicking a light with the light tool opens the editor; edits dispatch whole-payload writes with the raw stored emission as old", async () => {
  const { scene, tools } = captureScene();
  const dispatched: WireOperation[][] = [];
  render(ToolRail, {
    context: setAppContextForTest({ role: "gm", scene, documents: editorStore(), dispatchIntent: (ops) => dispatched.push(ops) }),
  });
  await fireEvent.click(screen.getByTestId("tool-light"));
  const tool = tools.at(-1)!;
  tool.onPointerDown({ x: 200, y: 200 }, {} as PointerEvent);
  tool.onPointerUp({ x: 200, y: 200 }, {} as PointerEvent);
  expect(await screen.findByTestId("light-editor")).toBeTruthy();

  const stored = { color: "#ffcc66", intensity: 0.8, brightRadius: 2, dimRadius: 6, falloff: null, enabled: true, rotation: 0, followTokenRotation: false };

  // Intensity edit: one `/engine/emission` write carrying the whole payload.
  await fireEvent.change(screen.getByTestId("emission-intensity"), { target: { value: "0.5" } });
  expect(dispatched.at(-1)![0]).toEqual({
    op: "update", doc_id: "light-1",
    changes: [{ path: "/engine/emission", old: stored, new: { ...stored, intensity: 0.5 } }],
  });

  // Enabled toggle off (suppress, not delete).
  await fireEvent.click(screen.getByTestId("emission-enabled"));
  expect(dispatched.at(-1)![0]).toEqual({
    op: "update", doc_id: "light-1",
    changes: [{ path: "/engine/emission", old: stored, new: { ...stored, enabled: false } }],
  });

  // Falloff select writes the wrapper object inside the whole payload; the raw stored
  // `falloff` was absent → null in both pre-image and payload.
  await fireEvent.change(screen.getByTestId("emission-falloff"), { target: { value: "quadratic" } });
  expect(dispatched.at(-1)![0]).toEqual({
    op: "update", doc_id: "light-1",
    changes: [{ path: "/engine/emission", old: stored, new: { ...stored, falloff: { curve: "quadratic" } } }],
  });
});

test("the light editor's delete dispatches the full pre-image and closes the editor", async () => {
  const { scene, tools } = captureScene();
  const dispatched: WireOperation[][] = [];
  render(ToolRail, {
    context: setAppContextForTest({ role: "gm", scene, documents: editorStore(), dispatchIntent: (ops) => dispatched.push(ops) }),
  });
  await fireEvent.click(screen.getByTestId("tool-light"));
  tools.at(-1)!.onPointerDown({ x: 200, y: 200 }, {} as PointerEvent);
  tools.at(-1)!.onPointerUp({ x: 200, y: 200 }, {} as PointerEvent);
  await screen.findByTestId("light-editor");
  await fireEvent.click(screen.getByTestId("light-delete"));
  const op = dispatched.at(-1)![0];
  expect(op.op).toBe("delete");
  if (op.op === "delete") expect(op.doc.id).toBe("light-1");
  expect(screen.queryByTestId("light-editor")).toBeNull();
});

test("selecting a wall with the select tool opens the flag editor; a flag toggle dispatches raw-old", async () => {
  const { scene, tools } = captureScene();
  const dispatched: WireOperation[][] = [];
  const docs = editorStore();
  render(ToolRail, {
    context: setAppContextForTest({ role: "gm", scene, documents: docs, dispatchIntent: (ops) => dispatched.push(ops) }),
  });
  await fireEvent.click(screen.getByTestId("tool-select"));
  tools.at(-1)!.onPointerDown({ x: 200, y: 700 }, { shiftKey: false } as PointerEvent);
  expect(await screen.findByTestId("wall-editor")).toBeTruthy();

  await fireEvent.click(screen.getByTestId("wall-blocks-light")); // uncheck blocksLight
  const wallId = docs.query("wall")[0].id;
  expect(dispatched.at(-1)![0]).toEqual({
    op: "update", doc_id: wallId,
    changes: [{ path: "/engine/blocksLight", old: true, new: false }],
  });
});

test("the wall editor's blocksSound checkbox authors an explicit override with raw-old", async () => {
  const { scene, tools } = captureScene();
  const dispatched: WireOperation[][] = [];
  const docs = editorStore();
  render(ToolRail, {
    context: setAppContextForTest({ role: "gm", scene, documents: docs, dispatchIntent: (ops) => dispatched.push(ops) }),
  });
  await fireEvent.click(screen.getByTestId("tool-select"));
  tools.at(-1)!.onPointerDown({ x: 200, y: 700 }, { shiftKey: false } as PointerEvent);
  expect(await screen.findByTestId("wall-editor")).toBeTruthy();
  const wallId = docs.query("wall")[0].id;

  // The fixture wall authors no `blocksSound` (unchecked; absent = inherit blocksSight
  // server-side), so checking it writes `true` with a null OCC pre-image.
  const box = screen.getByTestId("wall-blocks-sound") as HTMLInputElement;
  expect(box.checked).toBe(false);
  await fireEvent.click(box);
  expect(dispatched.at(-1)![0]).toEqual({
    op: "update", doc_id: wallId,
    changes: [{ path: "/engine/blocksSound", old: null, new: true }],
  });
});

test("the wall editor's door select dispatches /engine/door with the raw stored value as old", async () => {
  const { scene, tools } = captureScene();
  const dispatched: WireOperation[][] = [];
  const docs = editorStore();
  render(ToolRail, {
    context: setAppContextForTest({ role: "gm", scene, documents: docs, dispatchIntent: (ops) => dispatched.push(ops) }),
  });
  await fireEvent.click(screen.getByTestId("tool-select"));
  tools.at(-1)!.onPointerDown({ x: 200, y: 700 }, { shiftKey: false } as PointerEvent);
  expect(await screen.findByTestId("wall-editor")).toBeTruthy();

  // The fixture wall carries no door: the select reads "none".
  const wallId = docs.query("wall")[0].id;
  const select = screen.getByTestId("wall-door") as HTMLSelectElement;
  expect(select.value).toBe("none");

  // Authoring a closed door: old is the raw ABSENT value (null), per the OCC convention.
  await fireEvent.change(select, { target: { value: "closed" } });
  expect(dispatched.at(-1)![0]).toEqual({
    op: "update", doc_id: wallId,
    changes: [{ path: "/engine/door", old: null, new: "closed" }],
  });

  // Selecting "none" removes the door (writes null with the raw stored value as old).
  docs.applyCommand({
    seq: 2, world_id: "w1", author: "a", ts: 1,
    ops: [{
      op: "update", doc_id: wallId,
      changes: [{ path: "/engine/door", old: null, new: "closed" }],
    }],
  });
  // Let the store commit's re-render flush BEFORE the next change event, so the controlled
  // select value ("closed" now) settles before the event's own value assignment — otherwise
  // the flush lands between assignment and dispatch and the handler reads the stale value.
  await new Promise((r) => setTimeout(r, 0));
  await fireEvent.change(screen.getByTestId("wall-door"), { target: { value: "none" } });
  expect(dispatched.at(-1)![0]).toEqual({
    op: "update", doc_id: wallId,
    changes: [{ path: "/engine/door", old: "closed", new: null }],
  });
});

test("the wall editor's one-way and limited controls dispatch /engine/direction and /engine/sightTier with raw-old", async () => {
  const { scene, tools } = captureScene();
  const dispatched: WireOperation[][] = [];
  const docs = editorStore();
  render(ToolRail, {
    context: setAppContextForTest({ role: "gm", scene, documents: docs, dispatchIntent: (ops) => dispatched.push(ops) }),
  });
  await fireEvent.click(screen.getByTestId("tool-select"));
  tools.at(-1)!.onPointerDown({ x: 200, y: 700 }, { shiftKey: false } as PointerEvent);
  expect(await screen.findByTestId("wall-editor")).toBeTruthy();
  const wallId = docs.query("wall")[0].id;

  // The fixture wall carries no `direction`: the checkbox reads unchecked, and enabling it
  // writes `{ blockingSide: 1 }` with the raw ABSENT value (null) as old, per the OCC
  // convention.
  const oneWay = screen.getByTestId("wall-one-way") as HTMLInputElement;
  expect(oneWay.checked).toBe(false);
  expect(screen.queryByTestId("wall-flip-side")).toBeNull();
  await fireEvent.click(oneWay);
  expect(dispatched.at(-1)![0]).toEqual({
    op: "update", doc_id: wallId,
    changes: [{ path: "/engine/direction", old: null, new: { blockingSide: 1 } }],
  });

  // With a direction stored, the flip button negates the sign, carrying the raw stored
  // object as old. Flush the store commit's re-render first (the same settle discipline the
  // door-select case above needs).
  docs.applyCommand({
    seq: 3, world_id: "w1", author: "a", ts: 1,
    ops: [{
      op: "update", doc_id: wallId,
      changes: [{ path: "/engine/direction", old: null, new: { blockingSide: 1 } }],
    }],
  });
  await new Promise((r) => setTimeout(r, 0));
  await fireEvent.click(screen.getByTestId("wall-flip-side"));
  expect(dispatched.at(-1)![0]).toEqual({
    op: "update", doc_id: wallId,
    changes: [{ path: "/engine/direction", old: { blockingSide: 1 }, new: { blockingSide: -1 } }],
  });

  // The limited-sight checkbox writes the tier's wire spelling, raw-null as old.
  await fireEvent.click(screen.getByTestId("wall-limited"));
  expect(dispatched.at(-1)![0]).toEqual({
    op: "update", doc_id: wallId,
    changes: [{ path: "/engine/sightTier", old: null, new: "limited" }],
  });
});

test("Escape clears the open editor", async () => {
  const { scene, tools } = captureScene();
  render(ToolRail, { context: setAppContextForTest({ role: "gm", scene, documents: editorStore(), dispatchIntent: () => {} }) });
  await fireEvent.click(screen.getByTestId("tool-light"));
  tools.at(-1)!.onPointerDown({ x: 200, y: 200 }, {} as PointerEvent);
  tools.at(-1)!.onPointerUp({ x: 200, y: 200 }, {} as PointerEvent);
  await screen.findByTestId("light-editor");
  await fireEvent.keyDown(window, { key: "Escape" });
  expect(screen.queryByTestId("light-editor")).toBeNull();
});

test("the light editor's elevation input writes /engine/elevation, normalizing ground to null", async () => {
  const { scene, tools } = captureScene();
  const dispatched: WireOperation[][] = [];
  const docs = editorStore();
  render(ToolRail, {
    context: setAppContextForTest({ role: "gm", scene, documents: docs, dispatchIntent: (ops) => dispatched.push(ops) }),
  });
  await fireEvent.click(screen.getByTestId("tool-light"));
  tools.at(-1)!.onPointerDown({ x: 200, y: 200 }, {} as PointerEvent);
  tools.at(-1)!.onPointerUp({ x: 200, y: 200 }, {} as PointerEvent);
  await screen.findByTestId("light-editor");

  const input = screen.getByTestId("light-elevation") as HTMLInputElement;
  expect(input.value).toBe("0"); // absent stored value displays as ground
  await fireEvent.change(input, { target: { value: "10" } });
  expect(dispatched.at(-1)![0]).toEqual({
    op: "update", doc_id: "light-1",
    changes: [{ path: "/engine/elevation", old: null, new: 10 }],
  });
});

// --- Region trigger `Teleport` editor ---

/** Activates the region tool and adds one trigger row, ready for effect-type selection. */
async function openRegionTriggerRow(): Promise<void> {
  await fireEvent.click(screen.getByTestId("tool-region"));
  await fireEvent.click(screen.getByTestId("region-trigger-add"));
}

test("the trigger-event select offers every event the server fires, and the row binds the choice", async () => {
  const { scene } = captureScene();
  render(ToolRail, { context: setAppContextForTest({ role: "gm", scene }) });
  await openRegionTriggerRow();
  const select = screen.getByTestId("region-trigger-on") as HTMLSelectElement;
  expect(Array.from(select.options).map((o) => o.value)).toEqual([
    "enter",
    "arrest",
    "exit",
    "turn_start",
    "turn_end",
    "round_start",
    "round_end",
  ]);

  await fireEvent.change(select, { target: { value: "turn_start" } });
  expect(select.value).toBe("turn_start");
});

test("selecting teleport seeds the default target (same scene, origin, no elevation/vfx override)", async () => {
  const { scene } = captureScene();
  render(ToolRail, { context: setAppContextForTest({ role: "gm", scene }) });
  await openRegionTriggerRow();
  await fireEvent.change(screen.getByTestId("region-trigger-effect"), { target: { value: "teleport" } });

  expect((screen.getByTestId("region-trigger-teleport-x") as HTMLInputElement).value).toBe("0");
  expect((screen.getByTestId("region-trigger-teleport-y") as HTMLInputElement).value).toBe("0");
  expect((screen.getByTestId("region-trigger-teleport-elevation") as HTMLInputElement).value).toBe("");
});

test("editing scene/x/y/elevation/vfx updates the trigger row's target", async () => {
  const { scene } = captureScene();
  const vaultDoc = buildSceneDoc("w1", {}, "scene-2");
  render(ToolRail, {
    context: setAppContextForTest({
      role: "gm",
      scene,
      searchDocuments: (_q, _opts, onUpdate) => {
        onUpdate([{ document: vaultDoc, score: 0, snippet: "" }]);
        return Promise.resolve({ unsubscribe: () => {} });
      },
      pickAsset: (async () => "vfx-asset-1") as never,
    }),
  });
  await openRegionTriggerRow();
  await fireEvent.change(screen.getByTestId("region-trigger-effect"), { target: { value: "teleport" } });

  await fireEvent.input(screen.getByTestId("region-trigger-teleport-scene"), { target: { value: "vault" } });
  await fireEvent.click(screen.getByTestId("region-trigger-teleport-scene-hit"));
  await fireEvent.change(screen.getByTestId("region-trigger-teleport-x"), { target: { value: "50" } });
  await fireEvent.change(screen.getByTestId("region-trigger-teleport-y"), { target: { value: "75" } });
  await fireEvent.change(screen.getByTestId("region-trigger-teleport-elevation"), { target: { value: "12" } });
  await fireEvent.click(screen.getByTestId("region-trigger-teleport-vfx"));

  expect((screen.getByTestId("region-trigger-teleport-x") as HTMLInputElement).value).toBe("50");
  expect((screen.getByTestId("region-trigger-teleport-y") as HTMLInputElement).value).toBe("75");
  expect((screen.getByTestId("region-trigger-teleport-elevation") as HTMLInputElement).value).toBe("12");
  expect(screen.getByTestId("region-trigger-teleport-vfx").textContent).toContain("vfx-asset-1");
});

test("the teleport VFX clear button resets target.vfx to null and disappears until a VFX is picked again", async () => {
  const { scene } = captureScene();
  render(ToolRail, {
    context: setAppContextForTest({
      role: "gm", scene,
      pickAsset: (async () => "vfx-asset-1") as never,
    }),
  });
  await openRegionTriggerRow();
  await fireEvent.change(screen.getByTestId("region-trigger-effect"), { target: { value: "teleport" } });

  // No clear button until a VFX is picked.
  expect(screen.queryByTestId("region-trigger-teleport-vfx-clear")).toBeNull();

  await fireEvent.click(screen.getByTestId("region-trigger-teleport-vfx"));
  expect(screen.getByTestId("region-trigger-teleport-vfx").textContent).toContain("vfx-asset-1");
  const clearBtn = screen.getByTestId("region-trigger-teleport-vfx-clear");

  await fireEvent.click(clearBtn);
  expect(screen.getByTestId("region-trigger-teleport-vfx").textContent).not.toContain("vfx-asset-1");
  expect(screen.queryByTestId("region-trigger-teleport-vfx-clear")).toBeNull();
});

test("a persisted region document's triggers includes the authored Teleport effect verbatim", async () => {
  const { scene, tools } = captureScene();
  const dispatched: WireOperation[][] = [];
  render(ToolRail, {
    context: setAppContextForTest({
      role: "gm", scene, documents: sceneStore(),
      dispatchIntent: (ops) => dispatched.push(ops),
    }),
  });
  await openRegionTriggerRow();
  await fireEvent.change(screen.getByTestId("region-trigger-effect"), { target: { value: "teleport" } });
  await fireEvent.change(screen.getByTestId("region-trigger-teleport-x"), { target: { value: "50" } });
  await fireEvent.change(screen.getByTestId("region-trigger-teleport-y"), { target: { value: "75" } });

  const tool = tools.at(-1)!;
  tool.onPointerDown({ x: 0, y: 0 }, {} as PointerEvent);
  tool.onPointerUp({ x: 100, y: 100 }, {} as PointerEvent);

  const op = dispatched.at(-1)![0];
  expect(op.op).toBe("create");
  if (op.op === "create") {
    const engine = op.doc.engine as { triggers: Array<{ on: string; effect: unknown }> };
    expect(engine.triggers).toEqual([
      { on: "enter", effect: { type: "teleport", target: { scene: null, x: 50, y: 75, elevation: null, vfx: null } } },
    ]);
  }
});

test("beginPickPortalTarget switches ctx.setGmViewedScene and overrides the active tool; endPickPortalTarget restores both and writes the captured point", async () => {
  const { scene, tools } = captureScene();
  const gmViewedScenes: (string | null)[] = [];
  const vaultDoc = buildSceneDoc("w1", {}, "scene-2");
  render(ToolRail, {
    context: setAppContextForTest({
      role: "gm", scene, viewedSceneId: "s1",
      setGmViewedScene: (id) => gmViewedScenes.push(id),
      searchDocuments: (_q, _opts, onUpdate) => {
        onUpdate([{ document: vaultDoc, score: 0, snippet: "" }]);
        return Promise.resolve({ unsubscribe: () => {} });
      },
    }),
  });
  await openRegionTriggerRow();
  await fireEvent.change(screen.getByTestId("region-trigger-effect"), { target: { value: "teleport" } });
  // Author a destination scene distinct from the viewed one, so beginPickPortalTarget has
  // somewhere to roam to.
  await fireEvent.input(screen.getByTestId("region-trigger-teleport-scene"), { target: { value: "vault" } });
  await fireEvent.click(screen.getByTestId("region-trigger-teleport-scene-hit"));

  await fireEvent.click(screen.getByTestId("region-trigger-teleport-pick"));
  expect(screen.getByTestId("region-trigger-teleport-pick").textContent).toBe("tools.triggerTeleportPicking");
  // The roam call fires with the authored target scene, switching away from the viewed one.
  expect(gmViewedScenes).toEqual(["scene-2"]);
  const pickTool = tools.at(-1)!;
  expect(pickTool).not.toBeNull();

  pickTool!.onPointerDown({ x: 33, y: 44 }, {} as PointerEvent);
  await new Promise((r) => setTimeout(r, 0)); // drain the $state update microtask
  expect(screen.getByTestId("region-trigger-teleport-pick").textContent).toBe("tools.triggerTeleportPick");
  expect((screen.getByTestId("region-trigger-teleport-x") as HTMLInputElement).value).toBe("33");
  expect((screen.getByTestId("region-trigger-teleport-y") as HTMLInputElement).value).toBe("44");
  // The original viewed scene is restored once the pick ends.
  expect(gmViewedScenes).toEqual(["scene-2", "s1"]);
  // The tool active before the pick began (the region tool) is restored, not left null.
  expect(tools.at(-1)).not.toBe(pickTool);
});

test("the wall editor's band inputs write the whole /engine/elevation object, preserving the other end; both empty writes null", async () => {
  const { scene, tools } = captureScene();
  const dispatched: WireOperation[][] = [];
  const docs = editorStore();
  render(ToolRail, {
    context: setAppContextForTest({
      role: "gm", scene, documents: docs,
      dispatchIntent: (ops) => {
        dispatched.push(ops);
        // Apply the intent back so the next edit's `old` re-derives from confirmed state.
        docs.applyCommand({ seq: docs.appliedSeq + 1, world_id: "w1", author: "a", ts: 0, ops });
      },
    }),
  });
  await fireEvent.click(screen.getByTestId("tool-select"));
  tools.at(-1)!.onPointerDown({ x: 200, y: 700 }, { shiftKey: false } as PointerEvent);
  await screen.findByTestId("wall-editor");
  const wallId = docs.query("wall")[0].id;

  // Setting a bottom end on an unbounded (absent) band creates the band object.
  await fireEvent.change(screen.getByTestId("wall-elevation-bottom"), { target: { value: "2" } });
  expect(dispatched.at(-1)![0]).toEqual({
    op: "update", doc_id: wallId,
    changes: [{ path: "/engine/elevation", old: null, new: { bottom: 2, top: null } }],
  });

  // Setting the top end preserves the confirmed bottom.
  await fireEvent.change(screen.getByTestId("wall-elevation-top"), { target: { value: "10" } });
  expect(dispatched.at(-1)![0]).toEqual({
    op: "update", doc_id: wallId,
    changes: [{ path: "/engine/elevation", old: { bottom: 2, top: null }, new: { bottom: 2, top: 10 } }],
  });

  // Clearing both ends writes null — the canonical "occludes every elevation".
  await fireEvent.change(screen.getByTestId("wall-elevation-bottom"), { target: { value: "" } });
  expect(dispatched.at(-1)![0]).toEqual({
    op: "update", doc_id: wallId,
    changes: [{ path: "/engine/elevation", old: { bottom: 2, top: 10 }, new: { bottom: null, top: 10 } }],
  });
  await fireEvent.change(screen.getByTestId("wall-elevation-top"), { target: { value: "" } });
  expect(dispatched.at(-1)![0]).toEqual({
    op: "update", doc_id: wallId,
    changes: [{ path: "/engine/elevation", old: { bottom: null, top: 10 }, new: null }],
  });
});

test("selecting a region with the select tool opens its elevation editor; the band inputs write /engine/elevation", async () => {
  const { scene, tools } = captureScene();
  const dispatched: WireOperation[][] = [];
  render(ToolRail, {
    context: setAppContextForTest({ role: "gm", scene, documents: editorStore(), dispatchIntent: (ops) => dispatched.push(ops) }),
  });
  await fireEvent.click(screen.getByTestId("tool-select"));
  tools.at(-1)!.onPointerDown({ x: 1050, y: 1050 }, { shiftKey: false } as PointerEvent);
  expect(await screen.findByTestId("region-editor")).toBeTruthy();

  await fireEvent.change(screen.getByTestId("region-elevation-bottom"), { target: { value: "2" } });
  expect(dispatched.at(-1)![0]).toEqual({
    op: "update", doc_id: "region-1",
    changes: [{ path: "/engine/elevation", old: null, new: { bottom: 2, top: null } }],
  });
});

test("the region editor's delete dispatches the full pre-image and closes the editor", async () => {
  const { scene, tools } = captureScene();
  const dispatched: WireOperation[][] = [];
  render(ToolRail, {
    context: setAppContextForTest({ role: "gm", scene, documents: editorStore(), dispatchIntent: (ops) => dispatched.push(ops) }),
  });
  await fireEvent.click(screen.getByTestId("tool-select"));
  tools.at(-1)!.onPointerDown({ x: 1050, y: 1050 }, { shiftKey: false } as PointerEvent);
  await screen.findByTestId("region-editor");
  await fireEvent.click(screen.getByTestId("region-delete"));
  const op = dispatched.at(-1)![0];
  expect(op.op).toBe("delete");
  if (op.op === "delete") expect(op.doc.id).toBe("region-1");
  expect(screen.queryByTestId("region-editor")).toBeNull();
});

test("selecting a drawing with the select tool opens its elevation editor; the band inputs write /engine/elevation", async () => {
  const { scene, tools } = captureScene();
  const dispatched: WireOperation[][] = [];
  render(ToolRail, {
    context: setAppContextForTest({ role: "gm", scene, documents: editorStore(), dispatchIntent: (ops) => dispatched.push(ops) }),
  });
  await fireEvent.click(screen.getByTestId("tool-select"));
  tools.at(-1)!.onPointerDown({ x: 1350, y: 1350 }, { shiftKey: false } as PointerEvent);
  expect(await screen.findByTestId("drawing-editor")).toBeTruthy();

  await fireEvent.change(screen.getByTestId("drawing-elevation-bottom"), { target: { value: "3" } });
  expect(dispatched.at(-1)![0]).toEqual({
    op: "update", doc_id: "drawing-1",
    changes: [{ path: "/engine/elevation", old: null, new: { bottom: 3, top: null } }],
  });
});

test("the drawing editor's delete dispatches the full pre-image and closes the editor", async () => {
  const { scene, tools } = captureScene();
  const dispatched: WireOperation[][] = [];
  render(ToolRail, {
    context: setAppContextForTest({ role: "gm", scene, documents: editorStore(), dispatchIntent: (ops) => dispatched.push(ops) }),
  });
  await fireEvent.click(screen.getByTestId("tool-select"));
  tools.at(-1)!.onPointerDown({ x: 1350, y: 1350 }, { shiftKey: false } as PointerEvent);
  await screen.findByTestId("drawing-editor");
  await fireEvent.click(screen.getByTestId("drawing-delete"));
  const op = dispatched.at(-1)![0];
  expect(op.op).toBe("delete");
  if (op.op === "delete") expect(op.doc.id).toBe("drawing-1");
  expect(screen.queryByTestId("drawing-editor")).toBeNull();
});

test("selecting a template with the select tool opens its elevation editor; the band inputs write /engine/elevation", async () => {
  const { scene, tools } = captureScene();
  const dispatched: WireOperation[][] = [];
  render(ToolRail, {
    context: setAppContextForTest({ role: "gm", scene, documents: editorStore(), dispatchIntent: (ops) => dispatched.push(ops) }),
  });
  await fireEvent.click(screen.getByTestId("tool-select"));
  tools.at(-1)!.onPointerDown({ x: 1600, y: 1600 }, { shiftKey: false } as PointerEvent);
  expect(await screen.findByTestId("template-editor")).toBeTruthy();

  await fireEvent.change(screen.getByTestId("template-elevation-bottom"), { target: { value: "4" } });
  expect(dispatched.at(-1)![0]).toEqual({
    op: "update", doc_id: "template-1",
    changes: [{ path: "/engine/elevation", old: null, new: { bottom: 4, top: null } }],
  });
});

test("the template editor's delete dispatches the full pre-image and closes the editor", async () => {
  const { scene, tools } = captureScene();
  const dispatched: WireOperation[][] = [];
  render(ToolRail, {
    context: setAppContextForTest({ role: "gm", scene, documents: editorStore(), dispatchIntent: (ops) => dispatched.push(ops) }),
  });
  await fireEvent.click(screen.getByTestId("tool-select"));
  tools.at(-1)!.onPointerDown({ x: 1600, y: 1600 }, { shiftKey: false } as PointerEvent);
  await screen.findByTestId("template-editor");
  await fireEvent.click(screen.getByTestId("template-delete"));
  const op = dispatched.at(-1)![0];
  expect(op.op).toBe("delete");
  if (op.op === "delete") expect(op.doc.id).toBe("template-1");
  expect(screen.queryByTestId("template-editor")).toBeNull();
});

test("the rail renders one button per SCENE_TOOL_CONTRACT contribution and clicking it activates the contributed tool", async () => {
  const { scene, tools } = captureScene();
  const contributions = new ContributionRegistry();
  const clicks: Array<{ x: number; y: number }> = [];
  contributions.contribute({
    id: "test:fx",
    contract: SCENE_TOOL_CONTRACT,
    component: null,
    sceneTool: { id: "fx", icon: "✨", labelKey: "test.fxTool", onSceneClick: (x, y) => clicks.push({ x, y }) },
  });
  contributions.contribute({
    id: "test:other",
    contract: SCENE_TOOL_CONTRACT,
    component: null,
    sceneTool: { id: "other", icon: "🎯", labelKey: "test.otherTool", onSceneClick: () => {} },
  });
  render(ToolRail, { context: setAppContextForTest({ role: "gm", scene, contributions }) });

  const fx = screen.getByTestId("scene-tool-fx");
  expect(screen.getByTestId("scene-tool-other")).toBeTruthy();
  await fireEvent.click(fx);
  expect(fx.getAttribute("aria-pressed")).toBe("true");
  // The installed tool claims the click and forwards the scene point to onSceneClick.
  const tool = tools.at(-1);
  expect(tool).not.toBeNull();
  expect(tool!.onPointerDown({ x: 11, y: 22 }, {} as PointerEvent)).toBe(true);
  expect(clicks).toEqual([{ x: 11, y: 22 }]);
  // A built-in activation clears the contributed one (mutual exclusion).
  await fireEvent.click(screen.getByTestId("tool-ping"));
  expect(fx.getAttribute("aria-pressed")).toBe("false");
});

// The compound-region part-list gesture, driven through the REAL rail: arming via
// `region-part-add` switches the region tool's drags from immediate single-shape persists
// to staged parts (no document until commit), the per-row hole checkbox flips that part's
// flag, and `region-create-commit` folds the whole list into ONE compound region doc.
test("the compound region gesture stages parts via the rail and commits one compound region", async () => {
  const { scene, tools } = captureScene();
  const dispatched: WireOperation[][] = [];
  render(ToolRail, {
    context: setAppContextForTest({
      role: "gm",
      scene,
      documents: sceneStore(),
      dispatchIntent: (ops) => dispatched.push(ops),
    }),
  });
  await fireEvent.click(screen.getByTestId("tool-region"));
  const tool = tools.at(-1)!;
  const ev = {} as PointerEvent;

  // Arm the compound gesture, then drag two rects — nothing may persist yet.
  await fireEvent.click(screen.getByTestId("region-part-add"));
  tool.onPointerDown({ x: 0, y: 0 }, ev);
  tool.onPointerUp({ x: 100, y: 100 }, ev);
  await fireEvent.click(screen.getByTestId("region-part-add"));
  tool.onPointerDown({ x: 200, y: 200 }, ev);
  tool.onPointerUp({ x: 300, y: 300 }, ev);
  expect(dispatched).toEqual([]);

  // The rail lists both staged parts with a hole checkbox each; flip the second row's.
  await waitFor(() => expect(screen.getAllByTestId("region-part-hole")).toHaveLength(2));
  const holeBoxes = screen.getAllByTestId("region-part-hole");
  await fireEvent.click(holeBoxes[1]);

  await fireEvent.click(screen.getByTestId("region-create-commit"));
  expect(dispatched).toHaveLength(1);
  const op = dispatched[0][0];
  expect(op.op).toBe("create");
  if (op.op === "create") {
    expect(op.doc.doc_type).toBe("region");
    expect(op.doc.engine).toMatchObject({
      behavior: "terrain",
      shape: [
        { shape: { kind: "rect", points: [0, 0, 100, 100] }, hole: false },
        { shape: { kind: "rect", points: [200, 200, 300, 300] }, hole: true },
      ],
    });
  }
  // The gesture reset: the part rows are gone, and a plain drag persists immediately again.
  await waitFor(() => expect(screen.queryByTestId("region-part-hole")).toBeNull());
  tool.onPointerDown({ x: 0, y: 0 }, ev);
  tool.onPointerUp({ x: 50, y: 50 }, ev);
  expect(dispatched).toHaveLength(2);
  const single = dispatched[1][0];
  if (single.op === "create") {
    expect(single.doc.engine).toMatchObject({ shape: { kind: "rect", points: [0, 0, 50, 50] } });
  }
});
