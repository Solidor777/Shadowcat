import { describe, it, expect } from "vitest";
import { DocumentStore } from "@shadowcat/core";
import {
  MAX_BUBBLES,
  addBubble,
  resolveSpeakerToken,
  scanNewMessages,
  seedSeen,
  tokenInViewedScope,
  type Bubble,
} from "./bubbles";
import { addDocs, messageDoc, storeWith, tokenDoc } from "./__fixtures__/docs";

const tokenOwner = (id: string) => ({ kind: "token_instance", token_id: id });

describe("seedSeen + scanNewMessages", () => {
  it("a message present at mount never raises a bubble", () => {
    const store = storeWith("s1", messageDoc("m1", { actorOwner: tokenOwner("t1") }));
    const seen = new Set<string>();
    seedSeen(seen, store);
    expect(scanNewMessages(seen, store)).toEqual([]);
  });

  it("a message arriving after mount is returned once, then never again", () => {
    const store = storeWith("s1");
    const seen = new Set<string>();
    seedSeen(seen, store);
    addDocs(store, 2, messageDoc("m1", { actorOwner: tokenOwner("t1") }));
    expect(scanNewMessages(seen, store).map((m) => m.messageId)).toEqual(["m1"]);
    expect(scanNewMessages(seen, store)).toEqual([]);
  });

  it.each(["ooc", "system"])("a %s message raises no bubble but is still marked seen", (kind) => {
    const store = storeWith("s1");
    const seen = new Set<string>();
    addDocs(store, 2, messageDoc("m1", { kind, actorOwner: tokenOwner("t1") }));
    expect(scanNewMessages(seen, store)).toEqual([]);
    expect(seen.has("m1")).toBe(true);
  });

  it.each(["normal", "emote", "roll"])("a %s message spoken as a token is eligible", (kind) => {
    const store = storeWith("s1", messageDoc("m1", { kind, actorOwner: tokenOwner("t1") }));
    expect(scanNewMessages(new Set(), store).map((m) => m.messageId)).toEqual(["m1"]);
  });

  it("a message with no attribution, or already deleted, raises no bubble", () => {
    const store = storeWith(
      "s1",
      messageDoc("m1"),
      messageDoc("m2", { actorOwner: tokenOwner("t1"), deletedAt: 5 }),
    );
    expect(scanNewMessages(new Set(), store)).toEqual([]);
  });

  it("a malformed message body raises no bubble", () => {
    const bad = { ...messageDoc("m1"), engine: { not: "a message" } };
    const store = storeWith("s1", bad);
    expect(scanNewMessages(new Set(), store)).toEqual([]);
  });
});

describe("resolveSpeakerToken", () => {
  const store = (): DocumentStore =>
    storeWith(
      "s1",
      tokenDoc("t1", "s1", 0, 0, "a1"),
      tokenDoc("t2", "s1", 100, 0, "a2"),
      tokenDoc("t3", "s1", 200, 0, "a2"),
      tokenDoc("t-other", "s2", 0, 0, "a3"),
    );

  it("resolves a token_instance owner to its token when it is on the viewed scene", () => {
    expect(resolveSpeakerToken({ kind: "token_instance", token_id: "t1" }, store(), "s1", null)).toBe("t1");
  });

  it("a token on another scene does not resolve", () => {
    expect(resolveSpeakerToken({ kind: "token_instance", token_id: "t-other" }, store(), "s1", null)).toBeNull();
  });

  it("a missing token does not resolve", () => {
    expect(resolveSpeakerToken({ kind: "token_instance", token_id: "nope" }, store(), "s1", null)).toBeNull();
  });

  it("an actor owner resolves to its one placed token", () => {
    expect(resolveSpeakerToken({ kind: "actor", actor_id: "a1" }, store(), "s1", null)).toBe("t1");
  });

  it("an actor owner with two placed tokens is ambiguous and does not resolve", () => {
    expect(resolveSpeakerToken({ kind: "actor", actor_id: "a2" }, store(), "s1", null)).toBeNull();
  });

  it("an actor owner whose only token is on another scene does not resolve", () => {
    expect(resolveSpeakerToken({ kind: "actor", actor_id: "a3" }, store(), "s1", null)).toBeNull();
  });
});

describe("tokenInViewedScope", () => {
  it("is true on the viewed scene and false on another or with no token", () => {
    const s = storeWith("s1", tokenDoc("t1", "s1", 0, 0), tokenDoc("t2", "s2", 0, 0));
    expect(tokenInViewedScope("t1", s, "s1", null)).toBe(true);
    expect(tokenInViewedScope("t2", s, "s1", null)).toBe(false);
    expect(tokenInViewedScope("t1", s, "s2", null)).toBe(false);
  });
});

describe("addBubble", () => {
  const b = (messageId: string, tokenId: string): Bubble => ({ messageId, tokenId, channel: "general", content: [] });

  it("a newer bubble replaces the same token's older one", () => {
    const live = addBubble([b("m1", "t1"), b("m2", "t2")], b("m3", "t1"), MAX_BUBBLES);
    expect(live.map((x) => x.messageId)).toEqual(["m2", "m3"]);
  });

  it("evicts the oldest bubble past the cap", () => {
    let live: Bubble[] = [];
    for (let i = 0; i < MAX_BUBBLES + 3; i++) live = addBubble(live, b(`m${i}`, `t${i}`), MAX_BUBBLES);
    expect(live).toHaveLength(MAX_BUBBLES);
    expect(live[0].messageId).toBe("m3");
    expect(live.at(-1)?.messageId).toBe(`m${MAX_BUBBLES + 2}`);
  });
});
