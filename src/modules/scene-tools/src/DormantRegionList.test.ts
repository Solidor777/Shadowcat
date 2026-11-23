import { render, screen, fireEvent, cleanup } from "@testing-library/svelte";
import { test, expect, afterEach } from "vitest";
import { SceneInteractionBridge, TokenSelection } from "@shadowcat/ui-kit";
import { fakeSceneHost, setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore, buildSceneDoc, buildTokenDoc, buildRegionDoc, type RegionEngine, type WireOperation } from "@shadowcat/core";
import ToolRail from "./ToolRail.svelte";

afterEach(cleanup);

/** A region engine body anchored to `token` (or static when `token` is `null`). */
function regionEngine(token: string | null): RegionEngine {
  return {
    shape: { kind: "rect", points: [-150, -150, 150, 150] },
    behavior: "terrain", cost: 1, cost_by_tag: {}, enabled: true, triggers: [], elevation: null, ambient_override: null, occludes: null,
    anchor: token ? { token, include_carrier: true, follow_rotation: false } : null,
  };
}

/** A token body at `(x, y)`. */
function tokenEngine(x: number, y: number) {
  return { x, y, w: 100, h: 100, rotation: 0, visual: { kind: "image" as const, asset: "a" }, actor_id: null, overrides: null, face: null, elevation: null, lock_rotation: false, identity_hidden: false };
}

/** Scenes s1 and s2; on s1 a live carrier, a region following it, a region following a deleted
 * token (dormant), a static region, and a region following a token that sits on s2 (dormant: the
 * carrier is not on the region's scene); on s2 a token and a dormant region. */
function store(): DocumentStore {
  const docs = new DocumentStore();
  docs.applyCommand({
    seq: 1, world_id: "w1", author: "a", ts: 0,
    ops: [
      { op: "create", doc: buildSceneDoc("w1", {}, "s1") },
      { op: "create", doc: buildSceneDoc("w1", {}, "s2") },
      { op: "create", doc: buildTokenDoc("w1", "s1", tokenEngine(500, 500), "carrier-1") },
      { op: "create", doc: buildTokenDoc("w1", "s1", tokenEngine(800, 200), "spare-1") },
      { op: "create", doc: buildTokenDoc("w1", "s2", tokenEngine(100, 100), "other-scene-token") },
      { op: "create", doc: buildRegionDoc("w1", "s1", regionEngine("carrier-1"), "live0001") },
      { op: "create", doc: buildRegionDoc("w1", "s1", regionEngine("gone"), "gone0001") },
      { op: "create", doc: buildRegionDoc("w1", "s1", regionEngine(null), "stat0001") },
      { op: "create", doc: buildRegionDoc("w1", "s1", regionEngine("other-scene-token"), "else0001") },
      { op: "create", doc: buildRegionDoc("w1", "s2", regionEngine("gone"), "ons20001") },
    ],
  });
  return docs;
}

/** Renders the rail as a GM viewing `s1` with the region tool open. */
async function openRegionTool(docs: DocumentStore, selection = new TokenSelection(), dispatched: WireOperation[][] = []) {
  const scene = new SceneInteractionBridge();
  scene.attach(fakeSceneHost({}));
  render(ToolRail, {
    context: setAppContextForTest({ role: "gm", scene, documents: docs, viewedSceneId: "s1", tokenSelection: selection, dispatchIntent: (ops) => dispatched.push(ops) }),
  });
  await fireEvent.click(screen.getByTestId("tool-region"));
  return dispatched;
}

/** The region ids of the dormant rows, in order. */
function rowTexts(): string[] {
  return screen.queryAllByTestId("region-dormant-row").map((r) => r.getAttribute("data-region-id") ?? "");
}

test("lists exactly the viewed scene's dormant regions: a missing carrier and a carrier on another scene, not live, static or other-scene regions", async () => {
  await openRegionTool(store());
  const rows = rowTexts();
  expect(rows).toHaveLength(2);
  expect(rows.includes("gone0001")).toBe(true);
  expect(rows.includes("else0001")).toBe(true);
  for (const absent of ["live0001", "stat0001", "ons20001"]) expect(rows.join("|")).not.toContain(absent);
});

test("no dormant region renders no list", async () => {
  const docs = store();
  docs.applyCommand({ seq: 2, world_id: "w1", author: "a", ts: 0, ops: [{ op: "delete", doc: docs.get("gone0001")! }, { op: "delete", doc: docs.get("else0001")! }] });
  await openRegionTool(docs);
  expect(screen.queryByTestId("region-dormant-list")).toBeNull();
});

test("the list follows the store: deleting a carrier makes its region appear", async () => {
  const docs = store();
  await openRegionTool(docs);
  expect(rowTexts()).toHaveLength(2);
  docs.applyCommand({ seq: 2, world_id: "w1", author: "a", ts: 0, ops: [{ op: "delete", doc: docs.get("carrier-1")! }] });
  await screen.findAllByTestId("region-dormant-row");
  expect(rowTexts()).toHaveLength(3);
});

test("Delete dispatches a full pre-image delete of that region", async () => {
  const docs = store();
  const dispatched = await openRegionTool(docs);
  await fireEvent.click(screen.getAllByTestId("region-dormant-delete")[0]);
  const op = dispatched.at(-1)![0];
  expect(op.op).toBe("delete");
  if (op.op === "delete") expect(["gone0001", "else0001"]).toContain(op.doc.id);
});

test("Edit opens the region editor on the dormant region", async () => {
  await openRegionTool(store());
  expect(screen.queryByTestId("region-editor")).toBeNull();
  await fireEvent.click(screen.getAllByTestId("region-dormant-edit")[0]);
  expect(await screen.findByTestId("region-editor")).toBeTruthy();
});

test("Follow selected token is disabled until exactly one token on the region's scene is selected", async () => {
  const selection = new TokenSelection();
  await openRegionTool(store(), selection);
  const buttons = () => screen.getAllByTestId("region-dormant-reanchor") as HTMLButtonElement[];
  expect(buttons().every((b) => b.disabled)).toBe(true); // nothing selected
  selection.set(["carrier-1", "spare-1"]);
  await Promise.resolve();
  expect(buttons().every((b) => b.disabled)).toBe(true); // two selected
  selection.set(["other-scene-token"]);
  await Promise.resolve();
  expect(buttons().every((b) => b.disabled)).toBe(true); // a token on another scene
  selection.set(["spare-1"]);
  await Promise.resolve();
  expect(buttons().every((b) => !b.disabled)).toBe(true);
});

test("Follow selected token re-anchors the region to it, keeping its flags and stored offsets in one update", async () => {
  const selection = new TokenSelection();
  selection.set(["spare-1"]);
  const dispatched = await openRegionTool(store(), selection);
  const rowIndex = rowTexts().indexOf("gone0001");
  await fireEvent.click(screen.getAllByTestId("region-dormant-reanchor")[rowIndex]);
  const op = dispatched.at(-1)![0];
  expect(op.op).toBe("update");
  if (op.op !== "update") return;
  expect(op.changes).toEqual([
    { path: "/engine/anchor", old: { token: "gone", include_carrier: true, follow_rotation: false }, new: { token: "spare-1", include_carrier: true, follow_rotation: false } },
    { path: "/engine/shape", old: { kind: "rect", points: [-150, -150, 150, 150] }, new: { kind: "rect", points: [-150, -150, 150, 150] } },
  ]);
});

test("a player never sees the list (the region tool is GM-only)", async () => {
  const scene = new SceneInteractionBridge();
  scene.attach(fakeSceneHost({}));
  render(ToolRail, { context: setAppContextForTest({ role: "player", scene, documents: store(), viewedSceneId: "s1" }) });
  expect(screen.queryByTestId("tool-region")).toBeNull();
  expect(screen.queryByTestId("region-dormant-list")).toBeNull();
});
