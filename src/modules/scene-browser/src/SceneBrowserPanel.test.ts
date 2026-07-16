import { describe, it, expect } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import SceneBrowserPanel from "./SceneBrowserPanel.svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore, buildSceneDoc, buildWorldSettingsDoc, DEFAULT_WORLD_SETTINGS } from "@shadowcat/core";
import { SceneSelection } from "@shadowcat/ui-kit";

function seed(): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "u", ts: 0, ops: [
    { op: "create", doc: buildSceneDoc("w1", {}, "sA") },
    { op: "create", doc: buildSceneDoc("w1", {}, "sB") },
    { op: "create", doc: buildWorldSettingsDoc("w1", { ...structuredClone(DEFAULT_WORLD_SETTINGS), activeScene: "sA" }) },
  ] });
  return s;
}

describe("SceneBrowserPanel", () => {
  it("lists every scene", () => {
    const store = seed();
    const context = setAppContextForTest({ documents: store, store, role: "gm" });
    const { getAllByRole } = render(SceneBrowserPanel, { context });
    // one row per scene (each row has an Activate button)
    expect(getAllByRole("button", { name: /activate/i })).toHaveLength(2);
  });

  it("Activate writes world-settings.activeScene with the real pre-image as old", async () => {
    const store = seed();
    const sent: unknown[] = [];
    const context = setAppContextForTest({ documents: store, store, role: "gm", dispatchIntent: (ops) => sent.push(ops) });
    const { getAllByRole } = render(SceneBrowserPanel, { context });
    await fireEvent.click(getAllByRole("button", { name: /activate/i })[1]); // activate sB
    const op = (sent[0] as { op: string; doc_id: string; changes: { path: string; old: unknown; new: unknown }[] }[])[0];
    expect(op.op).toBe("update");
    expect(op.changes[0].path).toBe("/engine/activeScene");
    expect(op.changes[0].old).toBe("sA"); // REAL current value, not null
    expect(op.changes[0].new).toBe("sB");
  });

  it("View sets the GM local roam via ctx.setGmViewedScene", async () => {
    const store = seed();
    const roams: (string | null)[] = [];
    const context = setAppContextForTest({ documents: store, store, role: "gm", setGmViewedScene: (id) => roams.push(id) });
    const { getAllByRole } = render(SceneBrowserPanel, { context });
    // The default test-fixture `t()` returns the raw i18n key (identity stub, matching the
    // convention every other module test in this repo relies on, e.g. GameSettingsPanel's
    // getByLabelText("gameSettings.movementRestriction")) — match the key substring, not
    // translated English text.
    await fireEvent.click(getAllByRole("button", { name: /view/i })[1]);
    expect(roams).toEqual(["sB"]);
  });

  it("Configure focuses the scene in game-settings and opens it", async () => {
    const store = seed();
    const selection = new SceneSelection();
    const opened: string[] = [];
    const context = setAppContextForTest({
      documents: store, store, role: "gm", sceneSelection: selection,
      panels: { open: (id: string) => opened.push(id), close() {}, focus() {}, toggle() {}, minimized: [], metaMap: new Map(), restore() {} } as never,
    });
    const { getAllByRole } = render(SceneBrowserPanel, { context });
    await fireEvent.click(getAllByRole("button", { name: /configure/i })[1]);
    expect(selection.configureSceneId).toBe("sB");
    expect(opened).toEqual(["game-settings:panel"]);
  });

  it("Create dispatches a new scene document", async () => {
    const store = seed();
    const sent: unknown[] = [];
    const context = setAppContextForTest({ documents: store, store, role: "gm", world: "w1", dispatchIntent: (ops) => sent.push(ops) });
    const { getByRole } = render(SceneBrowserPanel, { context });
    await fireEvent.click(getByRole("button", { name: /create/i }));
    const op = (sent[0] as { op: string; doc: { doc_type: string } }[])[0];
    expect(op.op).toBe("create");
    expect(op.doc.doc_type).toBe("scene");
  });
});
