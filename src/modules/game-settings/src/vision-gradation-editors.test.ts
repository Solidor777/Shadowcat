import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import {
  DocumentStore,
  buildLightGradationDoc,
  buildVisionModesDoc,
  type LightGradationEngine,
  type VisionModesEngine,
  type WireDocument,
  type WireOperation,
} from "@shadowcat/core";
import GameSettingsPanel from "./GameSettingsPanel.svelte";

function storeWith(...docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((doc) => ({ op: "create" as const, doc })) });
  return s;
}

/** The seeded gradation doc's raw stored band array (unsorted, as stored). */
const seedBands = () => [
  { name: "bright", minIllumination: 0.67 },
  { name: "dim", minIllumination: 0.34 },
  { name: "dark", minIllumination: 0.0 },
];

function renderPanel(dispatchIntent: (ops: WireOperation[]) => void, docs: WireDocument[]) {
  return render(GameSettingsPanel, {
    context: setAppContextForTest({ role: "gm", world: "w1", documents: storeWith(...docs), dispatchIntent }),
  });
}

describe("gradation band editor", () => {
  it("add appends a uniquely-named band via a whole-array write with the raw stored bands as old", async () => {
    const dispatchIntent = vi.fn();
    renderPanel(dispatchIntent, [buildLightGradationDoc("w1", undefined, "lg1")]);
    await fireEvent.click(screen.getByLabelText("gameSettings.gradationAdd"));
    expect(dispatchIntent).toHaveBeenCalledWith([
      {
        op: "update",
        doc_id: "lg1",
        changes: [{ path: "/engine/bands", old: seedBands(), new: [...seedBands(), { name: "band-1", minIllumination: 0.5 }] }],
      },
    ]);
  });

  it("remove drops the band via a whole-array write", async () => {
    const dispatchIntent = vi.fn();
    renderPanel(dispatchIntent, [buildLightGradationDoc("w1", undefined, "lg1")]);
    await fireEvent.click(screen.getByLabelText("gameSettings.gradationRemove.dim"));
    expect(dispatchIntent).toHaveBeenCalledWith([
      {
        op: "update",
        doc_id: "lg1",
        changes: [{ path: "/engine/bands", old: seedBands(), new: [seedBands()[0], seedBands()[2]] }],
      },
    ]);
  });

  it("a threshold edit still writes the indexed pointer with the raw stored value as old", async () => {
    const dispatchIntent = vi.fn();
    renderPanel(dispatchIntent, [buildLightGradationDoc("w1", undefined, "lg1")]);
    await fireEvent.change(screen.getByLabelText("gameSettings.gradation.dark"), { target: { value: "0.1" } });
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "lg1", changes: [{ path: "/engine/bands/2/minIllumination", old: 0, new: 0.1 }] },
    ]);
  });
});

describe("vision-mode editor", () => {
  const seededModes = () => (buildVisionModesDoc("w1").engine as VisionModesEngine).modes;

  it("the floor dropdown options derive from the resolved gradation, not a hardcoded list", () => {
    const gradation: LightGradationEngine = { bands: [{ name: "gloom", minIllumination: 0.2 }, { name: "noon", minIllumination: 0.9 }] };
    renderPanel(vi.fn(), [buildLightGradationDoc("w1", gradation, "lg1"), buildVisionModesDoc("w1", undefined, "vm1")]);
    const select = screen.getByLabelText("gameSettings.visionMode.normal") as HTMLSelectElement;
    // resolveGradation sorts brightest-first; the seeded "dim"/"dark" floors are absent from a
    // custom gradation, so "dim" (normal's stored floor) appears once as the raw-value fallback.
    expect([...select.options].map((o) => o.value)).toEqual(["noon", "gloom", "dim"]);
  });

  it("floor, range, perceives, requiresLos, and name edits write per-field pointers with raw olds", async () => {
    const dispatchIntent = vi.fn();
    renderPanel(dispatchIntent, [buildLightGradationDoc("w1", undefined, "lg1"), buildVisionModesDoc("w1", undefined, "vm1")]);

    await fireEvent.change(screen.getByLabelText("gameSettings.visionMode.darkvision"), { target: { value: "dim" } });
    expect(dispatchIntent).toHaveBeenLastCalledWith([
      { op: "update", doc_id: "vm1", changes: [{ path: "/engine/modes/darkvision/illuminationFloor", old: "dark", new: "dim" }] },
    ]);

    await fireEvent.change(screen.getByLabelText("gameSettings.visionMode.darkvision.range"), { target: { value: "24" } });
    expect(dispatchIntent).toHaveBeenLastCalledWith([
      { op: "update", doc_id: "vm1", changes: [{ path: "/engine/modes/darkvision/defaultRange", old: 12, new: 24 }] },
    ]);

    await fireEvent.change(screen.getByLabelText("gameSettings.visionMode.tremorsense.perceives"), { target: { value: "terrain" } });
    expect(dispatchIntent).toHaveBeenLastCalledWith([
      { op: "update", doc_id: "vm1", changes: [{ path: "/engine/modes/tremorsense/perceives", old: "creatures", new: "terrain" }] },
    ]);

    await fireEvent.click(screen.getByLabelText("gameSettings.visionMode.tremorsense.requiresLos"));
    expect(dispatchIntent).toHaveBeenLastCalledWith([
      { op: "update", doc_id: "vm1", changes: [{ path: "/engine/modes/tremorsense/requiresLos", old: false, new: true }] },
    ]);

    await fireEvent.change(screen.getByLabelText("gameSettings.visionMode.normal.name"), { target: { value: "Normal sight" } });
    expect(dispatchIntent).toHaveBeenLastCalledWith([
      { op: "update", doc_id: "vm1", changes: [{ path: "/engine/modes/normal/name", old: "Normal", new: "Normal sight" }] },
    ]);
  });

  it("a render-hint edit writes the string, and emptying it writes null", async () => {
    const dispatchIntent = vi.fn();
    renderPanel(dispatchIntent, [buildVisionModesDoc("w1", undefined, "vm1")]);

    await fireEvent.change(screen.getByLabelText("gameSettings.visionMode.normal.renderHint"), { target: { value: "sepia" } });
    expect(dispatchIntent).toHaveBeenLastCalledWith([
      { op: "update", doc_id: "vm1", changes: [{ path: "/engine/modes/normal/renderHint", old: null, new: "sepia" }] },
    ]);

    await fireEvent.change(screen.getByLabelText("gameSettings.visionMode.darkvision.renderHint"), { target: { value: "" } });
    expect(dispatchIntent).toHaveBeenLastCalledWith([
      { op: "update", doc_id: "vm1", changes: [{ path: "/engine/modes/darkvision/renderHint", old: "desaturate", new: null }] },
    ]);
  });

  it("add creates a custom-N mode with a full descriptor literal (old: null); a second add picks the next free N", async () => {
    const dispatchIntent = vi.fn((ops: WireOperation[]) => {
      // Apply the intent back so the panel re-derives from the confirmed state.
      const change = ops[0];
      if (change.op === "update") {
        store.applyCommand({ seq: store.appliedSeq + 1, world_id: "w1", author: "a", ts: 0, ops: [change] });
      }
    });
    const store = storeWith(buildLightGradationDoc("w1", undefined, "lg1"), buildVisionModesDoc("w1", undefined, "vm1"));
    render(GameSettingsPanel, {
      context: setAppContextForTest({ role: "gm", world: "w1", documents: store, dispatchIntent }),
    });

    const expected = (id: string) => ({
      id, name: id, illuminationFloor: "dark", defaultRange: 12, perceives: "terrain", requiresLos: true, renderHint: null,
    });
    await fireEvent.click(screen.getByLabelText("gameSettings.visionModeAdd"));
    expect(dispatchIntent).toHaveBeenLastCalledWith([
      { op: "update", doc_id: "vm1", changes: [{ path: "/engine/modes/custom-1", old: null, new: expected("custom-1") }] },
    ]);
    await fireEvent.click(screen.getByLabelText("gameSettings.visionModeAdd"));
    expect(dispatchIntent).toHaveBeenLastCalledWith([
      { op: "update", doc_id: "vm1", changes: [{ path: "/engine/modes/custom-2", old: null, new: expected("custom-2") }] },
    ]);
  });

  it("remove replaces the whole modes map minus the removed id", async () => {
    const dispatchIntent = vi.fn();
    renderPanel(dispatchIntent, [buildVisionModesDoc("w1", undefined, "vm1")]);
    await fireEvent.click(screen.getByLabelText("gameSettings.visionModeRemove.tremorsense"));
    const next = seededModes();
    delete next.tremorsense;
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "vm1", changes: [{ path: "/engine/modes", old: seededModes(), new: next }] },
    ]);
  });

  it("a blank name edit dispatches nothing", async () => {
    const dispatchIntent = vi.fn();
    renderPanel(dispatchIntent, [buildVisionModesDoc("w1", undefined, "vm1")]);
    await fireEvent.change(screen.getByLabelText("gameSettings.visionMode.normal.name"), { target: { value: "   " } });
    expect(dispatchIntent).not.toHaveBeenCalled();
  });

  it("a mode entry predating the descriptor-v2 fields displays the server's defaults (terrain, LOS-gated)", () => {
    // A legacy stored entry lacks `perceives`/`requiresLos` keys entirely; the server resolves
    // them as `terrain`/`true` (the serde defaults), so the controls must display exactly that.
    const legacy = {
      id: "normal", name: "Normal", illuminationFloor: "dim", defaultRange: 0, renderHint: null,
    } as unknown as VisionModesEngine["modes"][string];
    renderPanel(vi.fn(), [buildVisionModesDoc("w1", { modes: { normal: legacy } }, "vm1")]);
    expect((screen.getByLabelText("gameSettings.visionMode.normal.requiresLos") as HTMLInputElement).checked).toBe(true);
    expect((screen.getByLabelText("gameSettings.visionMode.normal.perceives") as HTMLSelectElement).value).toBe("terrain");
  });
});
