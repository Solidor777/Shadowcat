import { render, screen, fireEvent } from "@testing-library/svelte";
import { test, expect } from "vitest";
import type { SceneTool } from "@shadowcat/render";
import { SceneInteractionBridge } from "@shadowcat/ui-kit";
import { fakeSceneHost } from "@shadowcat/ui-kit/test";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore, buildSceneDoc, buildTokenDoc, buildLightDoc, buildSceneEntityDoc, type WireOperation } from "@shadowcat/core";
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
        visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null, elevation: null,
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
        visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null, elevation: null,
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
      { x: 0, y: 0, w: 1, h: 1, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null, elevation: null },
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
      { x: 0, y: 0, w: 1, h: 1, rotation: 0, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null, elevation: null },
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
  const engine = { x: 0, y: 0, w: 1, h: 1, rotation: 0, visual: { kind: "image" as const, asset: "a" }, actor_id: null, overrides: null, face: null, elevation: null };
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

/** A store with one scene, one light at (200,200), one wall along y=700. */
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
          emission: { color: "#ffcc66", intensity: 0.8, brightRadius: 2, dimRadius: 6, falloff: null, enabled: true },
        }, "light-1"),
      },
      {
        op: "create",
        doc: buildSceneEntityDoc("w1", "s1", "wall", {
          seg: { x1: 0, y1: 700, x2: 400, y2: 700 }, blocksSight: true, blocksMove: true, blocksLight: true,
        }),
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

  const stored = { color: "#ffcc66", intensity: 0.8, brightRadius: 2, dimRadius: 6, falloff: null, enabled: true };

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
