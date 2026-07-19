import { describe, it, expect } from "vitest";
import { DocumentStore, ContributionRegistry, PANEL_CONTRACT, silentLogger, envelope, sheetContract } from "@shadowcat/core";
import { SheetsController } from "./sheetsController.svelte";
import SheetHost from "./SheetHost.svelte";
import type { PanelsApi } from "./panelsBridge.svelte";

function fakePanels(): PanelsApi & { opened: string[]; closed: string[]; focused: string[] } {
  const opened: string[] = [], closed: string[] = [], focused: string[] = [];
  return { opened, closed, focused, open: (id) => opened.push(id), close: (id) => closed.push(id), focus: (id) => focused.push(id), toggle: () => {} };
}

function seed() {
  const contributions = new ContributionRegistry();
  contributions.contribute({ id: "actor-sheet", contract: sheetContract("actor"), component: "ACTOR", sheet: { priority: 0 } }, { module: "sheet-actor" });
  const documents = new DocumentStore();
  documents.applyCommand({ seq: 1, world_id: "w1", author: "u", ts: 0, ops: [{ op: "create", doc: envelope("w1", "actor", null, { name: "A" }, "a1") }] });
  const panels = fakePanels();
  const ctrl = new SheetsController({ contributions, documents, panels, logger: silentLogger });
  return { contributions, documents, panels, ctrl };
}

describe("SheetsController.openDocument", () => {
  it("registers a sheet:<docId> panel contribution and opens it floating", () => {
    const { contributions, panels, ctrl } = seed();
    ctrl.openDocument({ docId: "a1" });
    const reg = contributions.contributionsFor(PANEL_CONTRACT).find((c) => c.id === "sheet:a1");
    expect(reg?.component).toBe(SheetHost);
    expect((reg?.props as { inner: unknown }).inner).toBe("ACTOR");
    expect(reg?.panel?.defaultPlacement).toEqual({ kind: "floating" });
    expect((reg?.props as { systemPrefix: string }).systemPrefix).toBe("/system");
    expect(panels.opened).toEqual(["sheet:a1"]);
  });

  it("focuses (does not re-register) an already-open document", () => {
    const { contributions, panels, ctrl } = seed();
    ctrl.openDocument({ docId: "a1" });
    ctrl.openDocument({ docId: "a1" });
    expect(contributions.contributionsFor(PANEL_CONTRACT).filter((c) => c.id === "sheet:a1")).toHaveLength(1);
    expect(panels.focused).toEqual(["sheet:a1"]);
  });

  it("logs and no-ops a dangling ref", () => {
    const { contributions, panels, ctrl } = seed();
    ctrl.openDocument({ docId: "gone" });
    expect(contributions.contributionsFor(PANEL_CONTRACT).some((c) => c.id.startsWith("sheet:"))).toBe(false);
    expect(panels.opened).toEqual([]);
  });

  it("closeDocument disposes the contribution and closes the panel", () => {
    const { contributions, panels, ctrl } = seed();
    ctrl.openDocument({ docId: "a1" });
    ctrl.closeDocument("sheet:a1");
    expect(contributions.contributionsFor(PANEL_CONTRACT).some((c) => c.id === "sheet:a1")).toBe(false);
    expect(panels.closed).toEqual(["sheet:a1"]);
  });
});

describe("SheetsController.restoreFromPersisted", () => {
  it("re-registers a resolvable sheet id found anywhere in the persisted blob, WITHOUT re-opening (panels restore its spot)", () => {
    const { contributions, panels, ctrl } = seed();
    ctrl.restoreFromPersisted({ expanded: { floating: [{ id: "sheet:a1", rect: {}, z: 0 }] } });
    expect(contributions.contributionsFor(PANEL_CONTRACT).some((c) => c.id === "sheet:a1")).toBe(true);
    expect(panels.opened).toEqual([]); // restoration is via late-registration, not open()
  });

  it("skips an unresolvable sheet id and is idempotent", () => {
    const { contributions, ctrl } = seed();
    const blob = { expanded: { minimized: ["sheet:gone", "sheet:a1"] } };
    ctrl.restoreFromPersisted(blob);
    ctrl.restoreFromPersisted(blob);
    const regs = contributions.contributionsFor(PANEL_CONTRACT).filter((c) => c.id.startsWith("sheet:"));
    expect(regs.map((r) => r.id)).toEqual(["sheet:a1"]);
  });

  it("re-registers an instanced token's self-describing embedded panel id (sheet:<tokenId>/embedded/actor/0)", () => {
    const { contributions, documents, panels, ctrl } = seed();
    const embedded = envelope("w1", "actor", null, { name: "Copy" }, "e1");
    const token = envelope("w1", "token", "sc1", { x: 0, y: 0 }, "t2");
    token.embedded = { actor: [embedded] };
    documents.applyCommand({ seq: 2, world_id: "w1", author: "u", ts: 0, ops: [{ op: "create", doc: token }] });
    ctrl.restoreFromPersisted({ expanded: { floating: [{ id: "sheet:t2/embedded/actor/0", rect: {}, z: 0 }] } });
    const reg = contributions.contributionsFor(PANEL_CONTRACT).find((c) => c.id === "sheet:t2/embedded/actor/0");
    expect(reg?.component).toBe(SheetHost);
    expect((reg?.props as { inner: unknown }).inner).toBe("ACTOR");
    expect(panels.opened).toEqual([]);
  });
});
