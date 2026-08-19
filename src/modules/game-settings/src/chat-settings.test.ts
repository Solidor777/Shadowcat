import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { DocumentStore, buildChatSettingsDoc, type ChatSettingsEngine, type WireDocument } from "@shadowcat/core";
import GameSettingsPanel from "./GameSettingsPanel.svelte";

function gmStoreWith(...docs: WireDocument[]) {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((doc) => ({ op: "create", doc })) });
  return s;
}

/** Full ChatSettingsEngine (every field required-nullable), overridable per test. */
function chatEngine(over: Partial<ChatSettingsEngine> = {}): ChatSettingsEngine {
  return { markdown: null, html: null, images: null, hyperlinks: null, emails: null, link_previews: null, ...over };
}

describe("chat settings editor", () => {
  it("toggling hyperlinks dispatches a JSON-pointer update with the real pre-image", async () => {
    const dispatchIntent = vi.fn();
    const chat = buildChatSettingsDoc("w1", chatEngine({ hyperlinks: false }), "chat1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(chat), dispatchIntent }) });

    const cb = screen.getByLabelText("gameSettings.chat.hyperlinks") as HTMLInputElement;
    await fireEvent.change(cb, { target: { checked: true } });

    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "chat1", changes: [{ path: "/engine/hyperlinks", old: false, new: true }] },
    ]);
  });

  it("toggling hyperlinks from a null stored value dispatches old: null, not old: false", async () => {
    const dispatchIntent = vi.fn();
    const chat = buildChatSettingsDoc("w1", chatEngine({ hyperlinks: null }), "chat1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(chat), dispatchIntent }) });

    const cb = screen.getByLabelText("gameSettings.chat.hyperlinks") as HTMLInputElement;
    await fireEvent.change(cb, { target: { checked: true } });

    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "chat1", changes: [{ path: "/engine/hyperlinks", old: null, new: true }] },
    ]);
  });

  it("hyperlinks checkbox reflects the stored value", () => {
    const dispatchIntent = vi.fn();
    const chat = buildChatSettingsDoc("w1", chatEngine({ hyperlinks: true }), "chat1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(chat), dispatchIntent }) });

    const cb = screen.getByLabelText("gameSettings.chat.hyperlinks") as HTMLInputElement;
    expect(cb.checked).toBe(true);
  });

  it("selecting 'Enabled' on link previews dispatches an explicit true with real pre-image null", async () => {
    const dispatchIntent = vi.fn();
    const chat = buildChatSettingsDoc("w1", chatEngine({ hyperlinks: true }), "chat1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(chat), dispatchIntent }) });

    const sel = screen.getByLabelText("gameSettings.chat.linkPreviews") as HTMLSelectElement;
    await fireEvent.change(sel, { target: { value: "true" } });

    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "chat1", changes: [{ path: "/engine/link_previews", old: null, new: true }] },
    ]);
  });

  it("selecting 'Disabled' on link previews dispatches an explicit false", async () => {
    const dispatchIntent = vi.fn();
    const chat = buildChatSettingsDoc("w1", chatEngine({ hyperlinks: true }), "chat1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(chat), dispatchIntent }) });

    const sel = screen.getByLabelText("gameSettings.chat.linkPreviews") as HTMLSelectElement;
    await fireEvent.change(sel, { target: { value: "false" } });

    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "chat1", changes: [{ path: "/engine/link_previews", old: null, new: false }] },
    ]);
  });

  it("selecting the default option after an explicit override writes null with the real pre-image", async () => {
    const dispatchIntent = vi.fn();
    const chat = buildChatSettingsDoc("w1", chatEngine({ hyperlinks: true, link_previews: false }), "chat1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(chat), dispatchIntent }) });

    const sel = screen.getByLabelText("gameSettings.chat.linkPreviews") as HTMLSelectElement;
    await fireEvent.change(sel, { target: { value: "" } });

    expect(dispatchIntent).toHaveBeenCalledWith([
      { op: "update", doc_id: "chat1", changes: [{ path: "/engine/link_previews", old: false, new: null }] },
    ]);
  });

  it("link previews select reflects an absent stored value as the default option", () => {
    const dispatchIntent = vi.fn();
    const chat = buildChatSettingsDoc("w1", chatEngine({ hyperlinks: true }), "chat1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(chat), dispatchIntent }) });

    const sel = screen.getByLabelText("gameSettings.chat.linkPreviews") as HTMLSelectElement;
    expect(sel.value).toBe("");
  });

  it("link previews select reflects an explicit true stored value", () => {
    const dispatchIntent = vi.fn();
    const chat = buildChatSettingsDoc("w1", chatEngine({ hyperlinks: true, link_previews: true }), "chat1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: gmStoreWith(chat), dispatchIntent }) });

    const sel = screen.getByLabelText("gameSettings.chat.linkPreviews") as HTMLSelectElement;
    expect(sel.value).toBe("true");
  });

  it("is not rendered for a non-GM", () => {
    const dispatchIntent = vi.fn();
    const chat = buildChatSettingsDoc("w1", chatEngine({ hyperlinks: true }), "chat1");
    render(GameSettingsPanel, { context: setAppContextForTest({ role: "player", world: "w1", documents: gmStoreWith(chat), dispatchIntent }) });

    expect(screen.queryByLabelText("gameSettings.chat.hyperlinks")).toBeNull();
    expect(screen.queryByLabelText("gameSettings.chat.linkPreviews")).toBeNull();
  });
});
