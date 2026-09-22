// @vitest-environment node
import { describe, it, expect } from "vitest";
import { HookBus } from "./hooks";
import { silentLogger } from "./logger";
import { defineChatComposerHooks, CHAT_COMPOSER_HOOK_VERSION, type ChatComposerSendPayload } from "./chat-hooks";

describe("defineChatComposerHooks", () => {
  it("declares chat:composer-send as a mutate-kind hook", async () => {
    const hooks = new HookBus(silentLogger);
    defineChatComposerHooks(hooks);
    const payload: ChatComposerSendPayload = { text: "hello", cancel: false };
    const result = await hooks.emitMutate("chat:composer-send", payload);
    // No listener registered: emitMutate returns the payload unchanged.
    expect(result).toEqual(payload);
  });

  it("is idempotent at the same version", () => {
    const hooks = new HookBus(silentLogger);
    expect(() => {
      defineChatComposerHooks(hooks);
      defineChatComposerHooks(hooks);
    }).not.toThrow();
  });

  it("a registered listener may transform text and set cancel", async () => {
    const hooks = new HookBus(silentLogger);
    defineChatComposerHooks(hooks);
    hooks.on("chat:composer-send", (p) => {
      const payload = p as ChatComposerSendPayload;
      return { ...payload, text: payload.text.toUpperCase() };
    });
    const result = await hooks.emitMutate("chat:composer-send", { text: "hi", cancel: false });
    expect(result).toEqual({ text: "HI", cancel: false });
  });

  it("a registered listener may cancel the send", async () => {
    const hooks = new HookBus(silentLogger);
    defineChatComposerHooks(hooks);
    hooks.on("chat:composer-send", (p) => ({ ...(p as ChatComposerSendPayload), cancel: true }));
    const result = await hooks.emitMutate("chat:composer-send", { text: "hi", cancel: false });
    expect(result.cancel).toBe(true);
  });
});

it("CHAT_COMPOSER_HOOK_VERSION is a semver string", () => {
  expect(CHAT_COMPOSER_HOOK_VERSION).toMatch(/^\d+\.\d+\.\d+$/);
});
