import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { ContributionRegistry } from "@shadowcat/core";
import { DocumentStore, buildChannelRegistryDoc, type WireDocument, type WireOperation } from "@shadowcat/core";
import ChatPanel from "./ChatPanel.svelte";
import Probe from "./__fixtures__/CardProbe.svelte";
import ComposerProbe from "./__fixtures__/ComposerProbe.svelte";

const cmd = (ops: WireOperation[]) => ({ seq: 1, world_id: "w1", author: "a", ts: 0, ops });
function storeWith(...docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand(cmd(docs.map((doc) => ({ op: "create" as const, doc }))));
  return s;
}
function msgDoc(id: string, created_at: number, system: Record<string, unknown>): WireDocument {
  return {
    id,
    scope: { kind: "world", world_id: "w1" },
    doc_type: "message",
    schema_version: 1,
    source: null,
    owner: "u1",
    permissions: { default: "observer", users: {} } as WireDocument["permissions"],
    embedded: {},
    parent_id: null,
    system,
    created_at,
    updated_at: created_at,
  };
}
const publicMsg = (id: string, created_at: number, channel = "general") =>
  msgDoc(id, created_at, { channel, user_owner: "u1", kind: "normal", audience: { kind: "public" }, content: [{ kind: "text", text: `msg-${id}` }] });
const gmMsg = (id: string, created_at: number) =>
  msgDoc(id, created_at, { channel: "general", user_owner: "u1", kind: "normal", audience: { kind: "gm_only" }, content: [{ kind: "text", text: `msg-${id}` }] });

function registryWithCard(): ContributionRegistry {
  const r = new ContributionRegistry();
  r.contribute({ id: "card", contract: "shadowcat.surface:chat.message", component: Probe });
  return r;
}
function registryWithCardAndComposer(): ContributionRegistry {
  const r = registryWithCard();
  r.contribute({ id: "composer", contract: "shadowcat.surface:chat.composer", component: ComposerProbe });
  return r;
}

describe("ChatPanel — rendering + view filters", () => {
  it("renders no card content when the chat.message surface has no contribution (graceful empty)", () => {
    const store = storeWith(publicMsg("m1", 1));
    render(ChatPanel, { context: setAppContextForTest({ role: "player", world: "w1", documents: store, store, contributions: new ContributionRegistry() }) });
    expect(screen.queryByTestId("card")).toBeNull();
  });

  it("renders seeded store messages through the stub card contribution with correct props", () => {
    const store = storeWith(publicMsg("m1", 1));
    render(ChatPanel, { context: setAppContextForTest({ role: "player", world: "w1", documents: store, store, contributions: registryWithCard() }) });
    const cards = screen.getAllByTestId("card");
    expect(cards.length).toBe(1);
    expect(cards[0].textContent).toContain("msg-m1");
    expect(cards[0].getAttribute("data-show-channel")).toBe("true"); // default view is "all"
  });

  it("an unknown-channel message still appears in the All view", () => {
    const store = storeWith(publicMsg("m1", 1, "some-unregistered-channel"));
    render(ChatPanel, { context: setAppContextForTest({ role: "player", world: "w1", documents: store, store, contributions: registryWithCard() }) });
    expect(screen.getAllByTestId("card").length).toBe(1);
  });

  it("switching to the GM pseudo-channel view shows only gm_only-audience messages", async () => {
    const store = storeWith(publicMsg("m1", 1), gmMsg("m2", 2));
    render(ChatPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: store, store, contributions: registryWithCard() }) });
    expect(screen.getAllByTestId("card").length).toBe(2);
    await fireEvent.click(screen.getByText("chat.gmChannel"));
    const cards = screen.getAllByTestId("card");
    expect(cards.length).toBe(1);
    expect(cards[0].textContent).toContain("msg-m2");
  });

  it("switching to a registered channel view filters by that channel only", async () => {
    const registry = buildChannelRegistryDoc("w1", { ooc: { name: "OOC" } }, "creg1");
    const store = storeWith(registry, publicMsg("m1", 1, "general"), publicMsg("m2", 2, "ooc"));
    render(ChatPanel, { context: setAppContextForTest({ role: "player", world: "w1", documents: store, store, contributions: registryWithCard() }) });
    await fireEvent.click(screen.getByText("OOC"));
    const cards = screen.getAllByTestId("card");
    expect(cards.length).toBe(1);
    expect(cards[0].textContent).toContain("msg-m2");
  });

  it("caps rendered messages to the last RENDER_CAP, slicing the oldest off", () => {
    const docs = Array.from({ length: 205 }, (_, i) => publicMsg(`m${i}`, i));
    const store = storeWith(...docs);
    render(ChatPanel, { context: setAppContextForTest({ role: "player", world: "w1", documents: store, store, contributions: registryWithCard() }) });
    const cards = screen.getAllByTestId("card");
    expect(cards.length).toBe(200);
    // Oldest 5 (m0..m4) are sliced off; the newest (m204) survives.
    expect(cards[0].textContent).toContain("msg-m5");
    expect(cards[cards.length - 1].textContent).toContain("msg-m204");
  });
});

describe("ChatPanel — GM channel registry seed", () => {
  it("seeds the channel registry once for GM when absent", async () => {
    const dispatchIntent = vi.fn();
    render(ChatPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent, contributions: new ContributionRegistry() }) });
    await vi.waitFor(() => expect(dispatchIntent).toHaveBeenCalled());
    const ops = dispatchIntent.mock.calls[0][0] as WireOperation[];
    expect(ops[0].op).toBe("create");
    const doc = (ops[0] as { doc: WireDocument }).doc;
    expect(doc.doc_type).toBe("channel-registry");
    expect((doc.system as { channels: Record<string, { name: string }> }).channels.general.name).toBe("General");
  });

  it("does not re-seed when a registry already exists", async () => {
    const dispatchIntent = vi.fn();
    const store = storeWith(buildChannelRegistryDoc("w1", { general: { name: "General" } }, "creg1"));
    render(ChatPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: store, store, dispatchIntent, contributions: new ContributionRegistry() }) });
    await Promise.resolve();
    expect(dispatchIntent.mock.calls.some((c) => (c[0] as WireOperation[])[0]?.op === "create")).toBe(false);
  });

  it("does not seed for a player", async () => {
    const dispatchIntent = vi.fn();
    render(ChatPanel, { context: setAppContextForTest({ role: "player", world: "w1", documents: new DocumentStore(), dispatchIntent, contributions: new ContributionRegistry() }) });
    await Promise.resolve();
    expect(dispatchIntent).not.toHaveBeenCalled();
  });
});

describe("ChatPanel — GM channel editor visibility", () => {
  it("GM sees the ⚙ editor toggle, player does not", () => {
    const gmRender = render(ChatPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), contributions: new ContributionRegistry() }) });
    expect(screen.getByLabelText("chat.channels.edit")).toBeTruthy();
    gmRender.unmount();

    const { queryByLabelText } = render(ChatPanel, { context: setAppContextForTest({ role: "player", world: "w1", documents: new DocumentStore(), contributions: new ContributionRegistry() }) });
    expect(queryByLabelText("chat.channels.edit")).toBeNull();
  });

  it("GM add-channel dispatches a single-key update at /system/channels/<uuid>", async () => {
    const dispatchIntent = vi.fn();
    const store = storeWith(buildChannelRegistryDoc("w1", { general: { name: "General" } }, "creg1"));
    render(ChatPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: store, store, dispatchIntent, contributions: new ContributionRegistry() }) });
    await fireEvent.click(screen.getByLabelText("chat.channels.edit"));
    // Two "chat.channels.name" inputs render: one per existing channel row (for
    // renaming) plus the add-new-channel field last — target it by placeholder.
    const nameInput = screen.getByPlaceholderText("chat.channels.name");
    await fireEvent.input(nameInput, { target: { value: "OOC" } });
    await fireEvent.click(screen.getByText("chat.channels.add"));
    expect(dispatchIntent).toHaveBeenCalledTimes(1);
    const ops = dispatchIntent.mock.calls[0][0] as WireOperation[];
    expect(ops[0].op).toBe("update");
    expect((ops[0] as { doc_id: string }).doc_id).toBe("creg1");
    const change = (ops[0] as { changes: { path: string; old: unknown; new: unknown }[] }).changes[0];
    expect(change.path).toMatch(/^\/system\/channels\/[0-9a-f-]+$/);
    expect(change.old).toBeNull();
    expect(change.new).toEqual({ name: "OOC" });
  });
});

describe("ChatPanel — composer instantiation", () => {
  it("the composer stub receives the correct postTarget props for the active view", async () => {
    const store = storeWith(buildChannelRegistryDoc("w1", { ooc: { name: "OOC" } }, "creg1"));
    render(ChatPanel, { context: setAppContextForTest({ role: "player", world: "w1", documents: store, store, contributions: registryWithCardAndComposer() }) });
    const composer = screen.getByTestId("composer");
    expect(composer.getAttribute("data-channel")).toBe("general");
    expect(composer.getAttribute("data-audience")).toBe("public");

    await fireEvent.click(screen.getByText("OOC"));
    expect(screen.getByTestId("composer").getAttribute("data-channel")).toBe("ooc");

    await fireEvent.click(screen.getByText("chat.gmChannel"));
    expect(screen.getByTestId("composer").getAttribute("data-audience")).toBe("gm_only");
  });
});
