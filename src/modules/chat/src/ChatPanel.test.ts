import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { ContributionRegistry } from "@shadowcat/core";
import { DocumentStore, buildChannelRegistryDoc, type WireDocument, type WireOperation } from "@shadowcat/core";
import ChatPanel from "./ChatPanel.svelte";
import Probe from "./__fixtures__/CardProbe.svelte";
import ComposerProbe from "./__fixtures__/ComposerProbe.svelte";
import { getParseCallCount, resetParseCallCount } from "./channels";
import { chatUnreadBadge } from "./unreadBadge";

const cmd = (ops: WireOperation[]) => ({ seq: 1, world_id: "w1", author: "a", ts: 0, ops });
function storeWith(...docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand(cmd(docs.map((doc) => ({ op: "create" as const, doc }))));
  return s;
}
function msgDoc(id: string, created_at: number, engine: Record<string, unknown>): WireDocument {
  return {
    id,
    scope: { kind: "world", world_id: "w1" },
    doc_type: "message",
    schema_version: 1,
    name: null,
    source: null,
    owner: "u1",
    permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null } as WireDocument["permissions"],
    embedded: {},
    parent_id: null,
    engine,
    system: {},
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

describe("ChatPanel — server-seeded channel registry", () => {
  it("never dispatches a registry create, even for a GM on an empty store (the server seeds it at world creation/join)", async () => {
    const dispatchIntent = vi.fn();
    render(ChatPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: new DocumentStore(), dispatchIntent, contributions: new ContributionRegistry() }) });
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

  it("GM add-channel dispatches a single-key update at /engine/channels/<uuid>", async () => {
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
    expect(change.path).toMatch(/^\/engine\/channels\/[0-9a-f-]+$/);
    expect(change.old).toBeNull();
    expect(change.new).toEqual({ name: "OOC" });
  });

  it("empty add-channel name falls back to the localized default name", async () => {
    const dispatchIntent = vi.fn();
    const store = storeWith(buildChannelRegistryDoc("w1", { general: { name: "General" } }, "creg1"));
    render(ChatPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: store, store, dispatchIntent, contributions: new ContributionRegistry() }) });
    await fireEvent.click(screen.getByLabelText("chat.channels.edit"));
    await fireEvent.click(screen.getByText("chat.channels.add"));
    const ops = dispatchIntent.mock.calls[0][0] as WireOperation[];
    const change = (ops[0] as { changes: { path: string; old: unknown; new: unknown }[] }).changes[0];
    expect(change.new).toEqual({ name: "chat.channels.newName" });
  });

  it("GM remove-channel dispatches a whole-field replace on /engine/channels with the key physically absent", async () => {
    const dispatchIntent = vi.fn();
    const channels = { general: { name: "General" }, ooc: { name: "OOC" } };
    const store = storeWith(buildChannelRegistryDoc("w1", channels, "creg1"));
    render(ChatPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: store, store, dispatchIntent, contributions: new ContributionRegistry() }) });
    await fireEvent.click(screen.getByLabelText("chat.channels.edit"));
    const removeButtons = screen.getAllByText("chat.channels.remove");
    await fireEvent.click(removeButtons[1]); // rows render in Object.entries order: general, ooc
    expect(dispatchIntent).toHaveBeenCalledTimes(1);
    const ops = dispatchIntent.mock.calls[0][0] as WireOperation[];
    expect(ops[0].op).toBe("update");
    expect((ops[0] as { doc_id: string }).doc_id).toBe("creg1");
    const change = (ops[0] as { changes: { path: string; old: unknown; new: unknown }[] }).changes[0];
    expect(change.path).toBe("/engine/channels");
    expect(change.old).toEqual(channels);
    expect(change.new).toEqual({ general: { name: "General" } });
    expect(Object.prototype.hasOwnProperty.call(change.new, "ooc")).toBe(false);
  });

  it("removing the LAST channel is refused client-side with a notification, no dispatch", async () => {
    const dispatchIntent = vi.fn();
    const notify = vi.fn();
    const store = storeWith(buildChannelRegistryDoc("w1", { general: { name: "General" } }, "creg1"));
    render(ChatPanel, { context: setAppContextForTest({ role: "gm", world: "w1", documents: store, store, dispatchIntent, notify, contributions: new ContributionRegistry() }) });
    await fireEvent.click(screen.getByLabelText("chat.channels.edit"));
    await fireEvent.click(screen.getByText("chat.channels.remove"));
    // An empty registry wedges all chat (ingest validates membership), so the
    // panel refuses before dispatching — the server would reject the write too.
    expect(dispatchIntent).not.toHaveBeenCalled();
    expect(notify).toHaveBeenCalledWith("chat.channels.lastChannel", "warning");
  });
});

describe("ChatPanel — composer instantiation", () => {
  it("the composer stub receives the correct postTarget props for the active view", async () => {
    const store = storeWith(buildChannelRegistryDoc("w1", { general: { name: "General" }, ooc: { name: "OOC" } }, "creg1"));
    render(ChatPanel, { context: setAppContextForTest({ role: "player", world: "w1", documents: store, store, contributions: registryWithCardAndComposer() }) });
    const composer = screen.getByTestId("composer");
    // The All view posts to the registry's lowest-sorted channel id ("general" < "ooc").
    expect(composer.getAttribute("data-channel")).toBe("general");
    expect(composer.getAttribute("data-audience")).toBe("public");

    await fireEvent.click(screen.getByText("OOC"));
    expect(screen.getByTestId("composer").getAttribute("data-channel")).toBe("ooc");

    await fireEvent.click(screen.getByText("chat.gmChannel"));
    expect(screen.getByTestId("composer").getAttribute("data-audience")).toBe("gm_only");
  });

  it("passes the channel's registered display name as placeholderName, not the sender's own username", async () => {
    const store = storeWith(buildChannelRegistryDoc("w1", { general: { name: "General" }, ooc: { name: "OOC" } }, "creg1"));
    render(ChatPanel, {
      context: setAppContextForTest({ role: "player", world: "w1", documents: store, store, selfId: "u1", contributions: registryWithCardAndComposer() }),
    });
    // All view posts to the "general" channel — placeholder is that channel's
    // display name, never members.get(selfId) (the sender's own username).
    expect(screen.getByTestId("composer").getAttribute("data-placeholder")).toBe("General");

    await fireEvent.click(screen.getByText("OOC"));
    expect(screen.getByTestId("composer").getAttribute("data-placeholder")).toBe("OOC");
  });

  it("falls back to the raw channel id when the post-target channel is unregistered", () => {
    render(ChatPanel, { context: setAppContextForTest({ role: "player", world: "w1", documents: new DocumentStore(), store: new DocumentStore(), contributions: registryWithCardAndComposer() }) });
    // No channel-registry doc exists yet: "general" has no registered name.
    expect(screen.getByTestId("composer").getAttribute("data-placeholder")).toBe("general");
  });

  it("the GM view's placeholderName prop is unaffected by placeholderGm ignoring it (real Composer branches on audience, not this prop)", async () => {
    const store = storeWith(buildChannelRegistryDoc("w1", { general: { name: "General" } }, "creg1"));
    render(ChatPanel, { context: setAppContextForTest({ role: "player", world: "w1", documents: store, store, contributions: registryWithCardAndComposer() }) });
    await fireEvent.click(screen.getByText("chat.gmChannel"));
    const composer = screen.getByTestId("composer");
    expect(composer.getAttribute("data-audience")).toBe("gm_only");
    // The GM postTarget channel is still "general", so placeholderName resolves
    // normally here too — it's the real Composer's placeholderGm branch
    // (gated on audience.kind, not on this prop) that makes it unused for GM.
    expect(composer.getAttribute("data-placeholder")).toBe("General");
  });
});

// jsdom has no layout engine: offsetParent, scrollHeight, and clientHeight all
// read as null/0 by default, so every test below stubs the properties it needs
// on the panel's own scroll container rather than relying on real layout.
function mockVisible(el: HTMLElement, visible: boolean): void {
  Object.defineProperty(el, "offsetParent", { configurable: true, get: () => (visible ? document.body : null) });
}
function mockScrollMetrics(el: HTMLElement, m: { scrollTop: number; clientHeight: number; scrollHeight: number }): void {
  Object.defineProperty(el, "scrollTop", { configurable: true, value: m.scrollTop, writable: true });
  Object.defineProperty(el, "clientHeight", { configurable: true, get: () => m.clientHeight });
  Object.defineProperty(el, "scrollHeight", { configurable: true, get: () => m.scrollHeight });
}

describe("ChatPanel — virtualization + narrowed re-derivation", () => {
  it("editing a message outside the current render window does not re-parse the full history", async () => {
    resetParseCallCount();
    const docs = Array.from({ length: 5000 }, (_, i) => publicMsg(`m${i}`, i));
    const store = storeWith(...docs);
    render(ChatPanel, { context: setAppContextForTest({ role: "player", world: "w1", documents: store, store, contributions: registryWithCard() }) });
    const countAfterMount = getParseCallCount();
    expect(countAfterMount).toBeGreaterThan(0); // the initial build does parse every message once

    // m10 is well outside the last-RENDER_CAP window (m4800..m4999).
    const oldContent = [{ kind: "text", text: "msg-m10" }];
    store.applyCommand(cmd([{ op: "update", doc_id: "m10", changes: [{ path: "/engine/content", old: oldContent, new: [{ kind: "text", text: "edited" }] }] }]));
    await Promise.resolve();

    expect(getParseCallCount()).toBe(countAfterMount); // no re-parse of any already-known message
  });

  it("the message list only mounts the measured scroll window plus overscan, not the full render-cap slice", () => {
    const docs = Array.from({ length: 5000 }, (_, i) => publicMsg(`m${i}`, i));
    const store = storeWith(...docs);
    const { container: root } = render(ChatPanel, { context: setAppContextForTest({ role: "player", world: "w1", documents: store, store, contributions: registryWithCard() }) });
    const messages = root.querySelector(".messages") as HTMLElement;
    // A real, disproportionate scroll geometry: the 200-message render-cap
    // slice would already satisfy a loose "<300" bound unwindowed, so this
    // asserts a much tighter bound only true windowing can meet.
    mockScrollMetrics(messages, { scrollTop: 0, clientHeight: 400, scrollHeight: 50000 });
    fireEvent.scroll(messages);
    const mountedRows = root.querySelectorAll("[data-message-row]");
    expect(mountedRows.length).toBeLessThan(60);
  });
});

describe("ChatPanel — new-message scroll/pill behavior", () => {
  it("scrolling up alone (no new message) never shows the new-messages pill", async () => {
    const store = storeWith(publicMsg("m1", 1));
    const { container: root } = render(ChatPanel, { context: setAppContextForTest({ role: "player", world: "w1", documents: store, store, contributions: registryWithCard() }) });
    const messages = root.querySelector(".messages") as HTMLElement;
    mockVisible(messages, true);
    mockScrollMetrics(messages, { scrollTop: 0, clientHeight: 100, scrollHeight: 1000 }); // far from bottom
    await fireEvent.scroll(messages);
    expect(screen.queryByText("chat.newMessages")).toBeNull();
  });

  it("a new message while scrolled up shows the pill instead of auto-scrolling", async () => {
    const store = storeWith(publicMsg("m1", 1));
    const { container: root } = render(ChatPanel, { context: setAppContextForTest({ role: "player", world: "w1", documents: store, store, contributions: registryWithCard() }) });
    const messages = root.querySelector(".messages") as HTMLElement;
    mockVisible(messages, true);
    mockScrollMetrics(messages, { scrollTop: 0, clientHeight: 100, scrollHeight: 1000 });
    await fireEvent.scroll(messages);
    store.applyCommand(cmd([{ op: "create", doc: publicMsg("m2", 2) }]));
    await vi.waitFor(() => expect(screen.queryByText("chat.newMessages")).toBeTruthy());
  });

  it("a new message while at the bottom auto-scrolls without showing the pill", async () => {
    const store = storeWith(publicMsg("m1", 1));
    const { container: root } = render(ChatPanel, { context: setAppContextForTest({ role: "player", world: "w1", documents: store, store, contributions: registryWithCard() }) });
    const messages = root.querySelector(".messages") as HTMLElement;
    mockVisible(messages, true);
    mockScrollMetrics(messages, { scrollTop: 900, clientHeight: 100, scrollHeight: 1000 }); // at bottom
    await fireEvent.scroll(messages);
    store.applyCommand(cmd([{ op: "create", doc: publicMsg("m2", 2) }]));
    await vi.waitFor(() => expect(messages.scrollTop).toBe(1000));
    expect(screen.queryByText("chat.newMessages")).toBeNull();
  });
});

describe("ChatPanel — hidden-tab scroll safety", () => {
  class FakeIntersectionObserver {
    static instances: FakeIntersectionObserver[] = [];
    callback: IntersectionObserverCallback;
    target?: Element;
    constructor(cb: IntersectionObserverCallback) {
      this.callback = cb;
      FakeIntersectionObserver.instances.push(this);
    }
    observe(target: Element): void {
      this.target = target;
    }
    unobserve(): void {}
    disconnect(): void {}
    trigger(isIntersecting: boolean): void {
      this.callback([{ isIntersecting, target: this.target } as IntersectionObserverEntry], this as unknown as IntersectionObserver);
    }
  }

  afterEach(() => {
    FakeIntersectionObserver.instances.length = 0;
    vi.unstubAllGlobals();
  });

  it("a message arriving while the tab is hidden writes no scrollTop; becoming visible scrolls to bottom", async () => {
    vi.stubGlobal("IntersectionObserver", FakeIntersectionObserver);
    const store = storeWith(publicMsg("m1", 1));
    const { container: root } = render(ChatPanel, { context: setAppContextForTest({ role: "player", world: "w1", documents: store, store, contributions: registryWithCard() }) });
    const messages = root.querySelector(".messages") as HTMLElement;
    // offsetParent defaults to null under jsdom, matching a display:none-hidden tab.
    Object.defineProperty(messages, "scrollHeight", { configurable: true, get: () => 500 });
    messages.scrollTop = 42; // sentinel prior position

    store.applyCommand(cmd([{ op: "create", doc: publicMsg("m2", 2) }]));
    await vi.waitFor(() => expect(screen.getAllByTestId("card").length).toBe(2));
    expect(messages.scrollTop).toBe(42); // unchanged: no write while hidden

    mockVisible(messages, true);
    FakeIntersectionObserver.instances[0]?.trigger(true);
    expect(messages.scrollTop).toBe(500);
  });

  it("a message arriving while hidden AND scrolled up keeps position on reveal and shows the pill, instead of force-scrolling to bottom", async () => {
    vi.stubGlobal("IntersectionObserver", FakeIntersectionObserver);
    const store = storeWith(publicMsg("m1", 1));
    const { container: root } = render(ChatPanel, { context: setAppContextForTest({ role: "player", world: "w1", documents: store, store, contributions: registryWithCard() }) });
    const messages = root.querySelector(".messages") as HTMLElement;
    // The initial mount (jsdom's default offsetParent is null, i.e. "hidden")
    // already deferred a scroll-to-bottom for m1 under the default atBottom=true
    // state. Consume that stale deferral first — reveal at the bottom — so it
    // cannot contaminate the scrolled-up scenario this test actually exercises.
    mockVisible(messages, true);
    mockScrollMetrics(messages, { scrollTop: 900, clientHeight: 100, scrollHeight: 1000 });
    FakeIntersectionObserver.instances[0]?.trigger(true);

    // Establish "scrolled up" (not at bottom) while still visible, then hide the tab.
    mockScrollMetrics(messages, { scrollTop: 0, clientHeight: 100, scrollHeight: 1000 });
    await fireEvent.scroll(messages);
    mockVisible(messages, false);
    Object.defineProperty(messages, "scrollHeight", { configurable: true, get: () => 500 });
    messages.scrollTop = 0; // sentinel prior (scrolled-up) position

    store.applyCommand(cmd([{ op: "create", doc: publicMsg("m2", 2) }]));
    await vi.waitFor(() => expect(screen.getAllByTestId("card").length).toBe(2));
    expect(messages.scrollTop).toBe(0); // unchanged: no write while hidden

    mockVisible(messages, true);
    FakeIntersectionObserver.instances[0]?.trigger(true);
    // Reader keeps their scrolled-up position — no force-scroll-to-bottom — and
    // gets the new-messages pill instead, mirroring the always-visible path.
    expect(messages.scrollTop).toBe(0);
    expect(screen.queryByText("chat.newMessages")).toBeTruthy();
  });
});

describe("ChatPanel — unread tab badge", () => {
  class FakeIntersectionObserver {
    static instances: FakeIntersectionObserver[] = [];
    callback: IntersectionObserverCallback;
    target?: Element;
    constructor(cb: IntersectionObserverCallback) {
      this.callback = cb;
      FakeIntersectionObserver.instances.push(this);
    }
    observe(target: Element): void {
      this.target = target;
    }
    unobserve(): void {}
    disconnect(): void {}
    trigger(isIntersecting: boolean): void {
      this.callback([{ isIntersecting, target: this.target } as IntersectionObserverEntry], this as unknown as IntersectionObserver);
    }
  }

  afterEach(() => {
    FakeIntersectionObserver.instances.length = 0;
    vi.unstubAllGlobals();
    chatUnreadBadge.set(0);
  });

  it("a message from another user arriving while the tab is hidden raises the badge", async () => {
    vi.stubGlobal("IntersectionObserver", FakeIntersectionObserver);
    const store = storeWith();
    render(ChatPanel, { context: setAppContextForTest({ role: "player", world: "w1", selfId: "self", documents: store, store, contributions: registryWithCard() }) });
    expect(chatUnreadBadge.get()).toBe(0);

    // offsetParent defaults to null under jsdom — matches a display:none-hidden tab.
    store.applyCommand(cmd([{ op: "create", doc: publicMsg("m1", 1) }])); // authored by u1, not "self"
    await vi.waitFor(() => expect(chatUnreadBadge.get()).toBe(1));
  });

  it("own messages arriving while hidden never raise the badge", async () => {
    vi.stubGlobal("IntersectionObserver", FakeIntersectionObserver);
    const store = storeWith();
    render(ChatPanel, { context: setAppContextForTest({ role: "player", world: "w1", selfId: "u1", documents: store, store, contributions: registryWithCard() }) });

    store.applyCommand(cmd([{ op: "create", doc: publicMsg("m1", 1) }])); // authored by u1 === selfId
    await Promise.resolve();
    expect(chatUnreadBadge.get()).toBe(0);
  });

  it("becoming visible clears the badge and persists the new read marker", async () => {
    vi.stubGlobal("IntersectionObserver", FakeIntersectionObserver);
    const setChatRead = vi.fn();
    const store = storeWith();
    const { container: root } = render(ChatPanel, {
      context: setAppContextForTest({
        role: "player",
        world: "w1",
        selfId: "self",
        documents: store,
        store,
        contributions: registryWithCard(),
        uiState: { getPanelLayout: () => null, setPanelLayout: () => {}, getChatRead: () => null, setChatRead },
      }),
    });
    const messages = root.querySelector(".messages") as HTMLElement;

    store.applyCommand(cmd([{ op: "create", doc: publicMsg("m1", 1) }]));
    await vi.waitFor(() => expect(chatUnreadBadge.get()).toBe(1));

    mockVisible(messages, true);
    FakeIntersectionObserver.instances[0]?.trigger(true);
    await vi.waitFor(() => expect(chatUnreadBadge.get()).toBe(0));
    expect(setChatRead).toHaveBeenCalledWith(expect.objectContaining({ general: { createdAt: 1, id: "m1" } }));
  });

  it("a persisted read marker from a prior session survives reload: only genuinely new messages count", async () => {
    vi.stubGlobal("IntersectionObserver", FakeIntersectionObserver);
    const store = storeWith(publicMsg("m1", 1), publicMsg("m2", 2));
    render(ChatPanel, {
      context: setAppContextForTest({
        role: "player",
        world: "w1",
        selfId: "self",
        documents: store,
        store,
        contributions: registryWithCard(),
        uiState: { getPanelLayout: () => null, setPanelLayout: () => {}, getChatRead: () => ({ general: { createdAt: 1, id: "m1" } }), setChatRead: () => {} },
      }),
    });
    await vi.waitFor(() => expect(chatUnreadBadge.get()).toBe(1)); // only m2 is newer than the persisted marker
  });
});
