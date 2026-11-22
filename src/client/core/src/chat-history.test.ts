// @vitest-environment node
import { describe, expect, it } from "vitest";
import { CHAT_HISTORY_BYTE_BUDGET, CHAT_HISTORY_LIMIT, parseChatHistory, pushChatHistory } from "./chat-history";

describe("pushChatHistory", () => {
  it("appends the newest entry last", () => {
    expect(pushChatHistory(["a"], "b")).toEqual(["a", "b"]);
  });

  it("evicts the oldest entries past the entry limit", () => {
    let history: string[] = [];
    for (let i = 0; i < CHAT_HISTORY_LIMIT + 5; i++) history = pushChatHistory(history, `m${i}`);
    expect(history).toHaveLength(CHAT_HISTORY_LIMIT);
    expect(history[0]).toBe("m5");
    expect(history[history.length - 1]).toBe(`m${CHAT_HISTORY_LIMIT + 4}`);
  });

  it("evicts the oldest entries until the serialized size fits the byte budget", () => {
    const big = "x".repeat(4000);
    let history: string[] = [];
    for (let i = 0; i < 20; i++) history = pushChatHistory(history, `${i}${big}`);
    expect(new TextEncoder().encode(JSON.stringify(history)).length).toBeLessThanOrEqual(CHAT_HISTORY_BYTE_BUDGET);
    expect(history[history.length - 1].startsWith("19")).toBe(true);
    expect(history.length).toBeLessThan(20);
  });

  it("counts UTF-8 bytes, not UTF-16 units, against the budget", () => {
    const wide = "😀".repeat(1000);
    let history: string[] = [];
    for (let i = 0; i < 10; i++) history = pushChatHistory(history, wide);
    expect(new TextEncoder().encode(JSON.stringify(history)).length).toBeLessThanOrEqual(CHAT_HISTORY_BYTE_BUDGET);
  });

  it("records nothing for an entry that alone exceeds the budget", () => {
    const huge = "\u0001".repeat(5000);
    expect(pushChatHistory(["a"], huge)).toEqual(["a"]);
  });

  it("does not mutate its input", () => {
    const before = ["a"];
    pushChatHistory(before, "b");
    expect(before).toEqual(["a"]);
  });
});

describe("parseChatHistory", () => {
  it("returns a stored string array unchanged", () => {
    expect(parseChatHistory(["a", "b"])).toEqual(["a", "b"]);
  });

  it("yields an empty list for null, a non-array or a malformed blob", () => {
    expect(parseChatHistory(null)).toEqual([]);
    expect(parseChatHistory({ a: 1 })).toEqual([]);
    expect(parseChatHistory("x")).toEqual([]);
  });

  it("drops non-string members", () => {
    expect(parseChatHistory(["a", 3, null, "b"])).toEqual(["a", "b"]);
  });
});
