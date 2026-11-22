import { describe, it, expect, afterEach, beforeEach, vi } from "vitest";
import { render } from "@testing-library/svelte";
import { flushSync } from "svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { StageView } from "@shadowcat/ui-kit";
import type { DocumentStore } from "@shadowcat/core";
import BubbleOverlay from "./BubbleOverlay.svelte";
import { BUBBLE_MS, MAX_BUBBLES } from "./bubbles";
import { ChatBubblesPreference } from "./preference.svelte";
import { addDocs, messageDoc, storeWith, tokenDoc } from "./__fixtures__/docs";

const tokenOwner = (id: string) => ({ kind: "token_instance", token_id: id });

/** Mounts the overlay over `store`, with the fog probe revealing every token unless `reveal` says otherwise. */
function mount(store: DocumentStore, opts: { enabled?: boolean; reveal?: (id: string) => boolean } = {}) {
  const stageView = new StageView();
  stageView.attachReveal((id) => (opts.reveal ? opts.reveal(id) : true));
  const preference = new ChatBubblesPreference(null);
  if (opts.enabled === false) preference.set(false);
  const result = render(BubbleOverlay, {
    props: { preference },
    context: setAppContextForTest({ documents: store, store, viewedSceneId: "s1", stageView }),
  });
  return { ...result, stageView, preference };
}

const bubbles = (c: HTMLElement) => [...c.querySelectorAll('[data-testid="chat-bubble"]')];

/** Lets the store emit and Svelte flush. */
async function settle(): Promise<void> {
  await Promise.resolve();
  flushSync();
}

describe("BubbleOverlay", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("a new message from a placed token raises one bubble above the token", async () => {
    const store = storeWith("s1", tokenDoc("t1", "s1", 500, 400));
    const { container } = mount(store);
    addDocs(store, 2, messageDoc("m1", { actorOwner: tokenOwner("t1"), text: "hi there" }));
    await settle();
    const found = bubbles(container as HTMLElement);
    expect(found).toHaveLength(1);
    expect(found[0].textContent).toContain("hi there");
    // Identity camera: scene (500, 400 - h/2) is the screen anchor.
    const style = (found[0] as HTMLElement).style;
    expect(style.left).toBe("500px");
    expect(style.top).toBe("350px");
  });

  it("history present at mount raises no bubble", async () => {
    const store = storeWith("s1", tokenDoc("t1", "s1", 0, 0), messageDoc("m1", { actorOwner: tokenOwner("t1") }));
    const { container } = mount(store);
    await settle();
    expect(bubbles(container as HTMLElement)).toHaveLength(0);
  });

  it("the toggle off suppresses bubbles and on does not replay the skipped message", async () => {
    const store = storeWith("s1", tokenDoc("t1", "s1", 0, 0));
    const { container, preference } = mount(store, { enabled: false });
    addDocs(store, 2, messageDoc("m1", { actorOwner: tokenOwner("t1") }));
    await settle();
    expect(bubbles(container as HTMLElement)).toHaveLength(0);
    preference.set(true);
    await settle();
    expect(bubbles(container as HTMLElement)).toHaveLength(0);
  });

  it("switching the toggle off hides a live bubble", async () => {
    const store = storeWith("s1", tokenDoc("t1", "s1", 0, 0));
    const { container, preference } = mount(store);
    addDocs(store, 2, messageDoc("m1", { actorOwner: tokenOwner("t1") }));
    await settle();
    expect(bubbles(container as HTMLElement)).toHaveLength(1);
    preference.set(false);
    await settle();
    expect(bubbles(container as HTMLElement)).toHaveLength(0);
  });

  it("a fogged token shows no bubble, and the bubble appears once the fog reveals it", async () => {
    const store = storeWith("s1", tokenDoc("t1", "s1", 0, 0));
    let revealed = false;
    const { container, stageView } = mount(store, { reveal: () => revealed });
    addDocs(store, 2, messageDoc("m1", { actorOwner: tokenOwner("t1") }));
    await settle();
    expect(bubbles(container as HTMLElement)).toHaveLength(0);
    revealed = true;
    stageView.noteMaskApplied();
    await settle();
    expect(bubbles(container as HTMLElement)).toHaveLength(1);
  });

  it("a speaker with no token on the viewed scene raises no bubble", async () => {
    const store = storeWith("s1", tokenDoc("t-other", "s2", 0, 0));
    const { container } = mount(store);
    addDocs(store, 2, messageDoc("m1", { actorOwner: tokenOwner("t-other") }), messageDoc("m2"));
    await settle();
    expect(bubbles(container as HTMLElement)).toHaveLength(0);
  });

  it("ooc and system messages raise no bubble", async () => {
    const store = storeWith("s1", tokenDoc("t1", "s1", 0, 0));
    const { container } = mount(store);
    addDocs(store, 2, messageDoc("m1", { kind: "ooc", actorOwner: tokenOwner("t1") }), messageDoc("m2", { kind: "system", actorOwner: tokenOwner("t1") }));
    await settle();
    expect(bubbles(container as HTMLElement)).toHaveLength(0);
  });

  it("a bubble expires after BUBBLE_MS", async () => {
    const store = storeWith("s1", tokenDoc("t1", "s1", 0, 0));
    const { container } = mount(store);
    addDocs(store, 2, messageDoc("m1", { actorOwner: tokenOwner("t1") }));
    await settle();
    expect(bubbles(container as HTMLElement)).toHaveLength(1);
    vi.advanceTimersByTime(BUBBLE_MS - 1);
    await settle();
    expect(bubbles(container as HTMLElement)).toHaveLength(1);
    vi.advanceTimersByTime(1);
    await settle();
    expect(bubbles(container as HTMLElement)).toHaveLength(0);
  });

  it("a second message from the same token replaces the first bubble", async () => {
    const store = storeWith("s1", tokenDoc("t1", "s1", 0, 0));
    const { container } = mount(store);
    addDocs(store, 2, messageDoc("m1", { actorOwner: tokenOwner("t1"), text: "first" }));
    await settle();
    addDocs(store, 3, messageDoc("m2", { actorOwner: tokenOwner("t1"), text: "second" }));
    await settle();
    const found = bubbles(container as HTMLElement);
    expect(found).toHaveLength(1);
    expect(found[0].textContent).toContain("second");
  });

  it("never holds more than MAX_BUBBLES bubbles", async () => {
    const count = MAX_BUBBLES + 4;
    const tokens = Array.from({ length: count }, (_, i) => tokenDoc(`t${i}`, "s1", i * 100, 0));
    const store = storeWith("s1", ...tokens);
    const { container } = mount(store);
    for (let i = 0; i < count; i++) {
      addDocs(store, 2 + i, messageDoc(`m${i}`, { actorOwner: tokenOwner(`t${i}`) }));
      await settle();
    }
    const found = bubbles(container as HTMLElement);
    expect(found).toHaveLength(MAX_BUBBLES);
    expect(found[0].getAttribute("data-token-id")).toBe("t4");
  });
});
