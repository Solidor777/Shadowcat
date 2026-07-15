import { describe, test, expect } from "vitest";
import { classifyDrop, opForMenuCommand, STAGE_ID, type DropSite } from "./policy";
import { defaultLayout } from "../layout/tree";

const layout = defaultLayout([]).expanded;

function isVeto(result: unknown): result is { veto: true; reason: string } {
  return typeof result === "object" && result !== null && (result as { veto?: unknown }).veto === true;
}

describe("classifyDrop — vetoes", () => {
  test("vetoes when the dragged subject is the stage panel", () => {
    const site: DropSite = { kind: "edge", id: STAGE_ID, position: "right" };
    const result = classifyDrop(site, layout);
    expect(isVeto(result)).toBe(true);
  });

  test("vetoes a drop targeting the stage's own group (kind: group)", () => {
    const site: DropSite = {
      kind: "group",
      id: "chat",
      position: "center",
      zone: "right",
      group: 0,
      stageGroup: true,
    };
    const result = classifyDrop(site, layout);
    expect(isVeto(result)).toBe(true);
  });

  test("vetoes a drop targeting the stage's own group (kind: edge)", () => {
    const site: DropSite = { kind: "edge", id: "chat", position: "right", stageGroup: true };
    const result = classifyDrop(site, layout);
    expect(isVeto(result)).toBe(true);
  });

  test("vetoes an edge drop resolving to 'top' (no top zone, spec D4)", () => {
    const site: DropSite = { kind: "edge", id: "chat", position: "top" };
    const result = classifyDrop(site, layout);
    expect(isVeto(result)).toBe(true);
    if (isVeto(result)) expect(result.reason).toMatch(/top/i);
  });

  test("vetoes an edge drop resolving to 'center'", () => {
    const site: DropSite = { kind: "edge", id: "chat", position: "center" };
    expect(isVeto(classifyDrop(site, layout))).toBe(true);
  });

  test("vetoes a malformed group drop missing zone/group", () => {
    const site: DropSite = { kind: "group", id: "chat", position: "center" };
    expect(isVeto(classifyDrop(site, layout))).toBe(true);
  });

  test("vetoes a group drop naming an unknown zone", () => {
    const site: DropSite = {
      kind: "group",
      id: "chat",
      position: "center",
      zone: "top" as never, // not a valid ZoneId; simulates a malformed translation
      group: 0,
    };
    expect(isVeto(classifyDrop(site, layout))).toBe(true);
  });

  test("vetoes a malformed floating drop missing a rect", () => {
    const site: DropSite = { kind: "floating", id: "chat", position: "center" };
    expect(isVeto(classifyDrop(site, layout))).toBe(true);
  });
});

describe("classifyDrop — happy paths", () => {
  test("edge-right resolves to a new group docked in zone 'right'", () => {
    const site: DropSite = { kind: "edge", id: "chat", position: "right" };
    expect(classifyDrop(site, layout)).toEqual({ op: "dock", id: "chat", zone: "right", group: "new" });
  });

  test("edge-left resolves to a new group docked in zone 'left'", () => {
    const site: DropSite = { kind: "edge", id: "chat", position: "left" };
    expect(classifyDrop(site, layout)).toEqual({ op: "dock", id: "chat", zone: "left", group: "new" });
  });

  test("edge-bottom resolves to a new group docked in zone 'bottom'", () => {
    const site: DropSite = { kind: "edge", id: "chat", position: "bottom" };
    expect(classifyDrop(site, layout)).toEqual({ op: "dock", id: "chat", zone: "bottom", group: "new" });
  });

  test("a tab-strip drop resolves to dock with a tabIndex", () => {
    const site: DropSite = {
      kind: "group",
      id: "chat",
      position: "center",
      zone: "right",
      group: 0,
      tabIndex: 2,
    };
    expect(classifyDrop(site, layout)).toEqual({ op: "dock", id: "chat", zone: "right", group: 0, tabIndex: 2 });
  });

  test("a group-content drop with no tabIndex docks without one", () => {
    const site: DropSite = { kind: "group", id: "chat", position: "center", zone: "right", group: 0 };
    expect(classifyDrop(site, layout)).toEqual({ op: "dock", id: "chat", zone: "right", group: 0 });
  });

  test("a float-drag resolves to float with the target rect", () => {
    const rect = { x: 10, y: 20, w: 300, h: 200 };
    const site: DropSite = { kind: "floating", id: "chat", position: "center", rect };
    expect(classifyDrop(site, layout)).toEqual({ op: "float", id: "chat", rect });
  });
});

describe("opForMenuCommand — parity with classifyDrop", () => {
  test("dockRight produces the exact op an edge-right drop would", () => {
    const site: DropSite = { kind: "edge", id: "chat", position: "right" };
    expect(opForMenuCommand("dockRight", "chat")).toEqual(classifyDrop(site, layout));
  });

  test("dockBottom produces the exact op an edge-bottom drop would", () => {
    const site: DropSite = { kind: "edge", id: "chat", position: "bottom" };
    expect(opForMenuCommand("dockBottom", "chat")).toEqual(classifyDrop(site, layout));
  });

  test("dockLeft produces the exact op an edge-left drop would", () => {
    const site: DropSite = { kind: "edge", id: "chat", position: "left" };
    expect(opForMenuCommand("dockLeft", "chat")).toEqual(classifyDrop(site, layout));
  });

  test("minimize/close are the bare LayoutOp — no drag equivalent to mirror", () => {
    expect(opForMenuCommand("minimize", "chat")).toEqual({ op: "minimize", id: "chat" });
    expect(opForMenuCommand("close", "chat")).toEqual({ op: "close", id: "chat" });
  });

  test("float carries the menu's fixed rect (no pointer position to mirror from a drag)", () => {
    const op = opForMenuCommand("float", "chat");
    expect(op).toMatchObject({ op: "float", id: "chat" });
    if ("op" in op && op.op === "float") expect(op.rect).toBeTruthy();
  });

  test("Finding 4: any command against the stage id is vetoed, mirroring classifyDrop's own stage veto", () => {
    for (const cmd of ["dockRight", "dockBottom", "dockLeft", "float", "minimize", "close"] as const) {
      const result = opForMenuCommand(cmd, STAGE_ID);
      expect("veto" in result && result.veto).toBe(true);
    }
  });
});
