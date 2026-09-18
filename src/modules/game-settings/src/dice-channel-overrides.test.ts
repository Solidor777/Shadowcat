import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { i18n } from "@shadowcat/ui-kit";
import { DocumentStore, buildDiceSettingsDoc, buildChannelRegistryDoc, type WireDocument } from "@shadowcat/core";
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

// Each channel row's controls are named by field AND channel ("Mode for channel general"), which
// only the catalog-backed `t` renders — the fixture's identity-echo `t` drops interpolation params,
// so every row would share one name.
const t = (k: string, p?: Parameters<typeof i18n.t>[1]) => i18n.t(k, p);

describe("per-channel dice-settings editor", () => {
  it("renders nothing when the channel registry has no channels", () => {
    const dispatchIntent = vi.fn();
    const dice = buildDiceSettingsDoc("w1", { mode: "total", direction: "high_wins", channel_overrides: {} }, "dice1");
    const reg = buildChannelRegistryDoc("w1", {}, "reg1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(dice, reg), dispatchIntent, t }) });

    expect(screen.queryByText("Channel overrides")).toBeNull();
  });

  it("renders without crashing when the stored doc predates channel_overrides (no key at all)", () => {
    // Simulates a dice-settings document created before this feature shipped:
    // the server's #[serde(default)] only fills the field when DESERIALIZING
    // into the typed Rust struct, never rewrites the stored JSON, and there
    // is no document-schema migration path — so the raw engine body can be
    // missing this key entirely. The client casts raw JSON with no runtime
    // validation (documented: GM-authored config, not untrusted wire input),
    // so any nested access must tolerate the key's genuine absence.
    const dispatchIntent = vi.fn();
    const dice = buildDiceSettingsDoc("w1", { mode: "total", direction: "high_wins", channel_overrides: {} }, "dice1");
    delete (dice.engine as { channel_overrides?: unknown }).channel_overrides;
    const reg = buildChannelRegistryDoc("w1", { general: { name: "General" } }, "reg1");

    expect(() =>
      render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(dice, reg), dispatchIntent, t }) }),
    ).not.toThrow();

    const sel = screen.getByLabelText("Custom settings for channel general") as HTMLSelectElement;
    expect(sel.value).toBe("");
  });

  it("renders one row per registered channel, defaulting to inherit", () => {
    const dispatchIntent = vi.fn();
    const dice = buildDiceSettingsDoc("w1", { mode: "total", direction: "high_wins", channel_overrides: {} }, "dice1");
    const reg = buildChannelRegistryDoc("w1", { general: { name: "General" }, ic: { name: "In Character" } }, "reg1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(dice, reg), dispatchIntent, t }) });

    const generalSel = screen.getByLabelText("Custom settings for channel general") as HTMLSelectElement;
    const icSel = screen.getByLabelText("Custom settings for channel ic") as HTMLSelectElement;
    expect(generalSel.value).toBe("");
    expect(icSel.value).toBe("");
    expect(screen.queryByLabelText("Mode for channel general")).toBeNull();
  });

  it("selecting Custom seeds mode/direction from the world default and dispatches a create", async () => {
    const dispatchIntent = vi.fn();
    const dice = buildDiceSettingsDoc("w1", { mode: "total", direction: "high_wins", channel_overrides: {} }, "dice1");
    const reg = buildChannelRegistryDoc("w1", { general: { name: "General" } }, "reg1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(dice, reg), dispatchIntent, t }) });

    const sel = screen.getByLabelText("Custom settings for channel general") as HTMLSelectElement;
    await fireEvent.change(sel, { target: { value: "override" } });

    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "dice1", changes: [{ path: "/engine/channel_overrides/general", old: null, new: { mode: "total", direction: "high_wins" } }] },
    ]);
  });

  it("editing mode on an existing override writes the FULL override object (full replacement)", async () => {
    const dispatchIntent = vi.fn();
    const dice = buildDiceSettingsDoc(
      "w1",
      { mode: "total", direction: "high_wins", channel_overrides: { general: { mode: "total", direction: "high_wins" } } },
      "dice1",
    );
    const reg = buildChannelRegistryDoc("w1", { general: { name: "General" } }, "reg1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(dice, reg), dispatchIntent, t }) });

    const modeSel = screen.getByLabelText("Mode for channel general") as HTMLSelectElement;
    await fireEvent.change(modeSel, { target: { value: "success_count" } });

    expect(dispatchIntent).toHaveBeenCalledWith([
      {
        op: "update", doc_id: "dice1",
        changes: [{
          path: "/engine/channel_overrides/general",
          old: { mode: "total", direction: "high_wins" },
          new: { mode: "success_count", direction: "high_wins" },
        }],
      },
    ]);
  });

  it("editing direction on an existing override writes the FULL override object", async () => {
    const dispatchIntent = vi.fn();
    const dice = buildDiceSettingsDoc(
      "w1",
      { mode: "total", direction: "high_wins", channel_overrides: { general: { mode: "total", direction: "high_wins" } } },
      "dice1",
    );
    const reg = buildChannelRegistryDoc("w1", { general: { name: "General" } }, "reg1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(dice, reg), dispatchIntent, t }) });

    const dirSel = screen.getByLabelText("Direction for channel general") as HTMLSelectElement;
    await fireEvent.change(dirSel, { target: { value: "low_wins" } });

    expect(dispatchIntent).toHaveBeenCalledWith([
      {
        op: "update", doc_id: "dice1",
        changes: [{
          path: "/engine/channel_overrides/general",
          old: { mode: "total", direction: "high_wins" },
          new: { mode: "total", direction: "low_wins" },
        }],
      },
    ]);
  });

  it("switching back to Inherit removes the key via a whole-map replace", async () => {
    const dispatchIntent = vi.fn();
    const dice = buildDiceSettingsDoc(
      "w1",
      { mode: "total", direction: "high_wins", channel_overrides: { general: { mode: "success_count", direction: "low_wins" } } },
      "dice1",
    );
    const reg = buildChannelRegistryDoc("w1", { general: { name: "General" } }, "reg1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(dice, reg), dispatchIntent, t }) });

    const sel = screen.getByLabelText("Custom settings for channel general") as HTMLSelectElement;
    await fireEvent.change(sel, { target: { value: "" } });

    expect(dispatchIntent).toHaveBeenCalledWith([
      {
        op: "update", doc_id: "dice1",
        changes: [{
          path: "/engine/channel_overrides",
          old: { general: { mode: "success_count", direction: "low_wins" } },
          new: {},
        }],
      },
    ]);
  });

  it("is not rendered for a non-GM", () => {
    const dispatchIntent = vi.fn();
    const dice = buildDiceSettingsDoc("w1", { mode: "total", direction: "high_wins", channel_overrides: {} }, "dice1");
    const reg = buildChannelRegistryDoc("w1", { general: { name: "General" } }, "reg1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "player", world: "w1", documents: gmStoreWith(dice, reg), dispatchIntent, t }) });

    expect(screen.queryByLabelText("Custom settings for channel general")).toBeNull();
  });
});
