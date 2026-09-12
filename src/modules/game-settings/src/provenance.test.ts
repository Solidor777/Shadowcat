import { render, screen, fireEvent } from "@testing-library/svelte";
import { describe, it, expect, vi } from "vitest";
import { DocumentStore, buildWorldSettingsDoc, buildSystemDefaultsDoc, deterministicId, SYSTEM_DEFAULTS_DOC_TYPE, type WireDocument } from "@shadowcat/core";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { i18n } from "@shadowcat/ui-kit";
import GameSettingsPanel from "./GameSettingsPanel.svelte";

// Suppress listAssets fetch: the panel's dice-sound picker calls listAssets in an $effect
// which hits /api/... in jsdom.
vi.mock("@shadowcat/core", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@shadowcat/core")>();
  return {
    ...actual,
    listAssets: vi.fn().mockResolvedValue([]),
  };
});

function gmStoreWith(...docs: WireDocument[]) {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((doc) => ({ op: "create", doc })) });
  return s;
}

// The reset button's accessible name carries the setting it resets, which only the catalog-backed
// `t` can render (the fixture's identity-echo `t` drops interpolation params), so these tests read
// the panel exactly as a user does.
const t = (k: string, p?: Parameters<typeof i18n.t>[1]) => i18n.t(k, p);

describe("settings provenance", () => {
  it("shows which layer supplies each world default", () => {
    const sd = buildSystemDefaultsDoc("w1", { scene: { fog: false } }, deterministicId("w1", SYSTEM_DEFAULTS_DOC_TYPE));
    // The world doc AUTHORS the fog leaf: structural provenance reports
    // "world" for exactly the leaves the overlay carries.
    const ws = buildWorldSettingsDoc("w1", { scene: { fog: true } }, "ws1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(sd, ws), dispatchIntent: vi.fn(), t }) });
    expect(screen.getByTestId("provenance:scene.fog").textContent).toContain("World setting");
  });

  it("reset clears the world leaf (writes null) so resolution falls through to the system layer", async () => {
    const dispatchIntent = vi.fn();
    const sd = buildSystemDefaultsDoc("w1", { scene: { fog: false } }, deterministicId("w1", SYSTEM_DEFAULTS_DOC_TYPE));
    const ws = buildWorldSettingsDoc("w1", { scene: { fog: true } }, "ws1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(sd, ws), dispatchIntent, t }) });
    await fireEvent.click(screen.getByLabelText("Reset Fog of war to system default"));
    // A CLEAR, never a client-resolved literal: null and absent are
    // wire-equivalent, so the leaf falls through to the system layer.
    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "ws1", changes: [{ path: "/engine/scene/fog", old: true, new: null }] },
    ]);
  });

  it("reports the system layer and renders no reset button when the world doc authors no leaf", () => {
    // The seeded world doc is the empty overlay: it authors nothing, so the
    // system layer supplies the value and there is no stored leaf to clear.
    const sd = buildSystemDefaultsDoc("w1", { scene: { fog: true } }, deterministicId("w1", SYSTEM_DEFAULTS_DOC_TYPE));
    const ws = buildWorldSettingsDoc("w1", undefined, "ws1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(sd, ws), dispatchIntent: vi.fn(), t }) });
    expect(screen.getByTestId("provenance:scene.fog").textContent).toContain("System default");
    expect(screen.queryByLabelText("Reset Fog of war to system default")).toBeNull();
  });
});
