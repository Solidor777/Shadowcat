import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import type { AppContext } from "@shadowcat/ui-kit";
import { SpeakAsToken } from "@shadowcat/ui-kit";
import { DocumentStore, buildActorDoc, buildChatSettingsDoc, MAX_MESSAGE_CHARS, type WireAudience, type WireCommand, type WireDocument, type ChatComposerSendPayload } from "@shadowcat/core";
import type { WorldRole } from "@shadowcat/types";
import Composer from "./Composer.svelte";

const publicAudience: WireAudience = { kind: "public" };
const gmAudience: WireAudience = { kind: "gm_only" };

const cmd = (ops: WireCommand["ops"]) => ({ seq: 1, world_id: "w1", author: "a", ts: 0, ops });
function storeWith(...docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand(cmd(docs.map((doc) => ({ op: "create" as const, doc }))));
  return s;
}

/** `buildActorDoc` seeds `owner: null`; tests that need an owned actor
 * set it directly, mirroring how the server stamps `owner` on Create. */
function ownedActor(id: string, owner: string, name: string): WireDocument {
  return { ...buildActorDoc("w1", name, { displayName: name, visual: { kind: "image", asset: "a1" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null, light: null, movement: [], aura: null, sound: null, vfx: null }, id), owner };
}

function renderComposer(
  opts: {
    audience?: WireAudience;
    send?: ReturnType<typeof vi.fn<(o: unknown) => Promise<void>>>;
    documents?: DocumentStore;
    role?: WorldRole;
    selfId?: string;
    searchDocuments?: AppContext["searchDocuments"];
    speakAsToken?: SpeakAsToken;
    pickAsset?: AppContext["pickAsset"];
    hooks?: AppContext["hooks"];
  } = {},
) {
  const send = opts.send ?? vi.fn<(o: unknown) => Promise<void>>(async () => {});
  const context = setAppContextForTest({
    chat: { send, edit: vi.fn(async () => {}), delete: vi.fn(async () => {}), recalc: vi.fn(async () => {}), reviseAudience: vi.fn(async () => {}), drawTable: vi.fn(async () => {}) },
    documents: opts.documents ?? new DocumentStore(),
    role: opts.role ?? "player",
    selfId: opts.selfId ?? "u-self",
    searchDocuments: opts.searchDocuments,
    speakAsToken: opts.speakAsToken,
    pickAsset: opts.pickAsset,
    hooks: opts.hooks,
  });
  render(Composer, { props: { channel: "general", audience: opts.audience ?? publicAudience, placeholderName: "Alice" }, context });
  return { send };
}

/** A world store seeded with a `chat-settings` singleton whose `images` toggle
 * is set to `enabled`. */
function storeWithImagesToggle(enabled: boolean): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand({
    seq: 1,
    world_id: "w1",
    author: "a",
    ts: 0,
    ops: [{
      op: "create",
      doc: buildChatSettingsDoc(
        "w1",
        { markdown: null, html: null, images: enabled, hyperlinks: null, emails: null, link_previews: null },
        "chat1",
      ),
    }],
  });
  return s;
}

describe("Composer — sending", () => {
  it("Enter sends trimmed content with the given channel/audience and clears", async () => {
    const { send } = renderComposer();
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    await fireEvent.input(textarea, { target: { value: "  hello world  " } });
    await fireEvent.keyDown(textarea, { key: "Enter" });
    expect(send).toHaveBeenCalledTimes(1);
    expect(send).toHaveBeenCalledWith({ channel: "general", content: "hello world", audience: publicAudience });
    expect(textarea.value).toBe("");
  });

  it("Shift+Enter does not send and inserts a newline instead", async () => {
    const { send } = renderComposer();
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    await fireEvent.input(textarea, { target: { value: "line one" } });
    await fireEvent.keyDown(textarea, { key: "Enter", shiftKey: true });
    expect(send).not.toHaveBeenCalled();
  });

  it("blocks sending empty content", async () => {
    const { send } = renderComposer();
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    await fireEvent.keyDown(textarea, { key: "Enter" });
    expect(send).not.toHaveBeenCalled();
  });

  it("blocks sending whitespace-only content", async () => {
    const { send } = renderComposer();
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    await fireEvent.input(textarea, { target: { value: "   \n  " } });
    await fireEvent.keyDown(textarea, { key: "Enter" });
    expect(send).not.toHaveBeenCalled();
  });

  it("blocks sending content over MAX_MESSAGE_CHARS and shows the counter", async () => {
    const { send } = renderComposer();
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    const overLong = "a".repeat(MAX_MESSAGE_CHARS + 1);
    await fireEvent.input(textarea, { target: { value: overLong } });
    await fireEvent.keyDown(textarea, { key: "Enter" });
    expect(send).not.toHaveBeenCalled();
    expect(screen.getByText("chat.composer.count")).toBeTruthy();
  });

  it("shows the counter once nearing the cap but not for short messages", async () => {
    const textarea = (() => {
      renderComposer();
      return screen.getByRole("textbox") as HTMLTextAreaElement;
    })();
    await fireEvent.input(textarea, { target: { value: "short" } });
    expect(screen.queryByText("chat.composer.count")).toBeNull();
    const nearCap = "a".repeat(MAX_MESSAGE_CHARS - 100);
    await fireEvent.input(textarea, { target: { value: nearCap } });
    expect(screen.getByText("chat.composer.count")).toBeTruthy();
  });

  it("passes a gm_only audience prop through verbatim to ctx.chat.send", async () => {
    const { send } = renderComposer({ audience: gmAudience });
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    await fireEvent.input(textarea, { target: { value: "secret" } });
    await fireEvent.keyDown(textarea, { key: "Enter" });
    expect(send).toHaveBeenCalledWith({ channel: "general", content: "secret", audience: gmAudience });
  });

  it("sends a slash-command body verbatim without any client-side parsing", async () => {
    const { send } = renderComposer();
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    await fireEvent.input(textarea, { target: { value: "/roll 1d6" } });
    await fireEvent.keyDown(textarea, { key: "Enter" });
    expect(send).toHaveBeenCalledWith({ channel: "general", content: "/roll 1d6", audience: publicAudience });
  });

  it("allows sending when trimmed length is under the cap even if raw padded length is over", async () => {
    const { send } = renderComposer();
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    const padding = " ".repeat(MAX_MESSAGE_CHARS);
    const paddedValue = `${padding}hello${padding}`;
    await fireEvent.input(textarea, { target: { value: paddedValue } });
    await fireEvent.keyDown(textarea, { key: "Enter" });
    expect(send).toHaveBeenCalledWith({ channel: "general", content: "hello", audience: publicAudience });
  });

  it("does not send on Enter while an IME composition is in progress", async () => {
    const { send } = renderComposer();
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    await fireEvent.input(textarea, { target: { value: "こんにちは" } });
    await fireEvent.keyDown(textarea, { key: "Enter", isComposing: true });
    expect(send).not.toHaveBeenCalled();
  });

  it("clicking Send also sends", async () => {
    const { send } = renderComposer();
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    await fireEvent.input(textarea, { target: { value: "via button" } });
    await fireEvent.click(screen.getByText("chat.composer.send"));
    expect(send).toHaveBeenCalledWith({ channel: "general", content: "via button", audience: publicAudience });
  });

  it("surfaces a server rejection inline instead of the message silently vanishing", async () => {
    const send = vi
      .fn<(o: unknown) => Promise<void>>()
      .mockRejectedValue(new Error("Rate limit exceeded, try again shortly."));
    renderComposer({ send });
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    await fireEvent.input(textarea, { target: { value: "spam" } });
    await fireEvent.keyDown(textarea, { key: "Enter" });
    expect(send).toHaveBeenCalledTimes(1);
    // Cleared optimistically, then restored once the rejection resolves (the composer
    // sat empty in between, so the draft is not lost) alongside the surfaced error.
    expect(await screen.findByText(/Rate limit exceeded/i)).toBeTruthy();
    expect(textarea.value).toBe("spam");
  });

  it("restores the sent draft into the composer when the send rejects and nothing new was typed", async () => {
    let rejectSend!: (e: Error) => void;
    const send = vi.fn<(o: unknown) => Promise<void>>(
      () => new Promise<void>((_resolve, reject) => { rejectSend = reject; }),
    );
    renderComposer({ send });
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    await fireEvent.input(textarea, { target: { value: "spam" } });
    await fireEvent.keyDown(textarea, { key: "Enter" });
    // Optimistic clear fires immediately; the send itself is still in flight.
    expect(textarea.value).toBe("");
    rejectSend(new Error("Rate limit exceeded, try again shortly."));
    await screen.findByText(/Rate limit exceeded/i);
    // The composer sat empty the whole time — the rejected draft is restored.
    expect(textarea.value).toBe("spam");
  });

  it("keeps newly typed text when the send rejects, never overwriting it with the failed draft", async () => {
    let rejectSend!: (e: Error) => void;
    const send = vi.fn<(o: unknown) => Promise<void>>(
      () => new Promise<void>((_resolve, reject) => { rejectSend = reject; }),
    );
    renderComposer({ send });
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    await fireEvent.input(textarea, { target: { value: "spam" } });
    await fireEvent.keyDown(textarea, { key: "Enter" });
    expect(textarea.value).toBe("");
    // The author starts a fresh message before the rejection surfaces.
    await fireEvent.input(textarea, { target: { value: "already typing something new" } });
    rejectSend(new Error("Rate limit exceeded, try again shortly."));
    await screen.findByText(/Rate limit exceeded/i);
    expect(textarea.value).toBe("already typing something new");
  });

  it("an earlier send's rejection never restores its draft over a later send, and the later rejection still restores", async () => {
    const rejectors: Array<(e: Error) => void> = [];
    const send = vi.fn<(o: unknown) => Promise<void>>(
      () => new Promise<void>((_resolve, reject) => { rejectors.push(reject); }),
    );
    renderComposer({ send });
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    await fireEvent.input(textarea, { target: { value: "first" } });
    await fireEvent.keyDown(textarea, { key: "Enter" });
    await fireEvent.input(textarea, { target: { value: "second" } });
    await fireEvent.keyDown(textarea, { key: "Enter" });
    expect(send).toHaveBeenCalledTimes(2);
    expect(textarea.value).toBe("");
    // The first send rejects after the second send's own optimistic clear: the box is empty,
    // but it is empty because of the LATER send, so the stale first draft stays out of it.
    rejectors[0](new Error("First send rejected."));
    await screen.findByText(/First send rejected/i);
    expect(textarea.value).toBe("");
    // The second, latest send rejecting restores its own draft.
    rejectors[1](new Error("Second send rejected."));
    await screen.findByText(/Second send rejected/i);
    expect(textarea.value).toBe("second");
  });

  it("an earlier send's rejection never restores its draft while a later send is still in its pre-send hook", async () => {
    const rejectors: Array<(e: Error) => void> = [];
    const send = vi.fn<(o: unknown) => Promise<void>>(
      () => new Promise<void>((_resolve, reject) => { rejectors.push(reject); }),
    );
    let resolveSecondHook!: (p: ChatComposerSendPayload) => void;
    let hookCalls = 0;
    const emitMutate = vi.fn(async (_name: string, payload: ChatComposerSendPayload) => {
      hookCalls += 1;
      if (hookCalls === 1) return payload;
      return new Promise<ChatComposerSendPayload>((resolve) => { resolveSecondHook = resolve; });
    });
    const hooks = { emitMutate } as unknown as AppContext["hooks"];
    renderComposer({ send, hooks });
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    await fireEvent.input(textarea, { target: { value: "first" } });
    await fireEvent.keyDown(textarea, { key: "Enter" });
    expect(send).toHaveBeenCalledTimes(1);
    // The second attempt starts and parks in its hook; the author then empties the box.
    await fireEvent.input(textarea, { target: { value: "second" } });
    await fireEvent.keyDown(textarea, { key: "Enter" });
    await fireEvent.input(textarea, { target: { value: "" } });
    // The first send rejects while the second attempt is still in its hook: the box is empty,
    // but a later attempt has begun, so the stale first draft stays out of it.
    rejectors[0](new Error("First send rejected."));
    await screen.findByText(/First send rejected/i);
    expect(textarea.value).toBe("");
    resolveSecondHook({ text: "second", cancel: false });
    await vi.waitFor(() => expect(send).toHaveBeenCalledTimes(2));
    expect(textarea.value).toBe("");
  });

  it("an earlier send's rejection never restores its draft after a later attempt is vetoed and the box emptied", async () => {
    const rejectors: Array<(e: Error) => void> = [];
    const send = vi.fn<(o: unknown) => Promise<void>>(
      () => new Promise<void>((_resolve, reject) => { rejectors.push(reject); }),
    );
    let hookCalls = 0;
    const emitMutate = vi.fn(async (_name: string, payload: ChatComposerSendPayload) => {
      hookCalls += 1;
      if (hookCalls === 1) return payload;
      return { ...payload, cancel: true };
    });
    const hooks = { emitMutate } as unknown as AppContext["hooks"];
    renderComposer({ send, hooks });
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    await fireEvent.input(textarea, { target: { value: "first" } });
    await fireEvent.keyDown(textarea, { key: "Enter" });
    expect(send).toHaveBeenCalledTimes(1);
    // The second attempt is vetoed by its hook: it claims a generation but never dispatches,
    // and a veto leaves its own draft in the box.
    await fireEvent.input(textarea, { target: { value: "second" } });
    await fireEvent.keyDown(textarea, { key: "Enter" });
    await vi.waitFor(() => expect(emitMutate).toHaveBeenCalledTimes(2));
    expect(send).toHaveBeenCalledTimes(1);
    expect(textarea.value).toBe("second");
    // The author empties the box by hand; the first send's rejection then surfaces into an
    // empty box, but a later attempt has begun, so the stale first draft stays out of it.
    await fireEvent.input(textarea, { target: { value: "" } });
    rejectors[0](new Error("First send rejected."));
    await screen.findByText(/First send rejected/i);
    expect(textarea.value).toBe("");
  });

  // The in-flight guard is scoped to the `chat:composer-send` hook's own await, never to
  // `ctx.chat.send`'s returned promise — an ordinary accepted chat op settles that promise only
  // on the wire client's silence timeout (no ack frame exists for a successful send), so a guard
  // held open across it would disable Send for that whole window on every ordinary message. A
  // pending HOOK is what these two tests hold open instead.
  it("a second Enter before the pre-send hook resolves does not invoke the hook or send twice", async () => {
    let resolveHook!: (p: ChatComposerSendPayload) => void;
    const emitMutate = vi.fn(
      async () => new Promise<ChatComposerSendPayload>((resolve) => { resolveHook = resolve; }),
    );
    const hooks = { emitMutate } as unknown as AppContext["hooks"];
    const { send } = renderComposer({ hooks });
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    await fireEvent.input(textarea, { target: { value: "first" } });
    await fireEvent.keyDown(textarea, { key: "Enter" });
    // The pre-send hook is still pending: a second rapid Enter must be dropped by the
    // in-flight guard, not queue a second hook call.
    await fireEvent.input(textarea, { target: { value: "second" } });
    await fireEvent.keyDown(textarea, { key: "Enter" });
    expect(emitMutate).toHaveBeenCalledTimes(1);
    resolveHook({ text: "first", cancel: false });
    await waitFor(() => expect(send).toHaveBeenCalledTimes(1));
  });

  it("the Send button stays disabled while the pre-send hook is still pending, even once new text is typed", async () => {
    let resolveHook!: (p: ChatComposerSendPayload) => void;
    const hooks: AppContext["hooks"] = {
      emitMutate: (async () =>
        new Promise<ChatComposerSendPayload>((resolve) => { resolveHook = resolve; })) as unknown as AppContext["hooks"]["emitMutate"],
    };
    renderComposer({ hooks });
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    await fireEvent.input(textarea, { target: { value: "first" } });
    const button = screen.getByText("chat.composer.send") as HTMLButtonElement;
    expect(button.disabled).toBe(false);
    await fireEvent.click(button);
    // `canSend` alone would allow this — the button must stay disabled on the in-flight guard.
    await fireEvent.input(textarea, { target: { value: "second" } });
    expect(button.disabled).toBe(true);
    resolveHook({ text: "first", cancel: false });
    // The guard releases once the hook settles and the send is dispatched — it does not wait
    // for `ctx.chat.send`'s own promise (see the two tests' shared comment above).
    await waitFor(() => expect(button.disabled).toBe(false));
  });

  it("a SYNCHRONOUS throw from ctx.chat.send still surfaces the error and clears the in-flight guard", async () => {
    // A `.finally()` chained only onto the promise `ctx.chat.send` returns can never run when
    // the call itself throws before returning a promise at all — the guarded region's own
    // `try`/`finally` is what closes that gap. A synchronous throw happens before the optimistic
    // clear, so the draft is left in place (not silently discarded) — the author's next Enter
    // resubmits the same text.
    const send = vi.fn(() => {
      throw new Error("Synchronous failure.");
    });
    renderComposer({ send: send as unknown as ReturnType<typeof vi.fn<(o: unknown) => Promise<void>>> });
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    await fireEvent.input(textarea, { target: { value: "boom" } });
    await fireEvent.keyDown(textarea, { key: "Enter" });
    expect(send).toHaveBeenCalledTimes(1);
    expect(await screen.findByText(/Synchronous failure/i)).toBeTruthy();
    // The in-flight guard cleared (not stuck `true` forever): the Send button is enabled again,
    // since `canSend` and `!sending` both hold — a second, real attempt is possible.
    const button = screen.getByText("chat.composer.send") as HTMLButtonElement;
    expect(button.disabled).toBe(false);
    await fireEvent.keyDown(textarea, { key: "Enter" });
    expect(send).toHaveBeenCalledTimes(2);
  });

  it("clears a prior rejection notice once the author starts typing again", async () => {
    const send = vi
      .fn<(o: unknown) => Promise<void>>()
      .mockRejectedValue(new Error("Message is too long."));
    renderComposer({ send });
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    await fireEvent.input(textarea, { target: { value: "x" } });
    await fireEvent.keyDown(textarea, { key: "Enter" });
    expect(await screen.findByText(/too long/i)).toBeTruthy();
    await fireEvent.input(textarea, { target: { value: "y" } });
    expect(screen.queryByText(/too long/i)).toBeNull();
  });

  it("keeps text typed while a slow chat:composer-send hook is in flight, instead of discarding it on the optimistic clear", async () => {
    let resolveHook!: (p: ChatComposerSendPayload) => void;
    const hooks: AppContext["hooks"] = {
      emitMutate: (async (_name: string, payload: unknown) =>
        new Promise<ChatComposerSendPayload>((resolve) => {
          resolveHook = resolve;
          void payload;
        })) as unknown as AppContext["hooks"]["emitMutate"],
    };
    const { send } = renderComposer({ hooks });
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    await fireEvent.input(textarea, { target: { value: "first draft" } });
    await fireEvent.keyDown(textarea, { key: "Enter" });
    // The hook is still pending — the author keeps typing before it resolves.
    await fireEvent.input(textarea, { target: { value: "first draft, more" } });
    resolveHook({ text: "first draft", cancel: false });
    await waitFor(() => expect(send).toHaveBeenCalledWith({ channel: "general", content: "first draft", audience: publicAudience }));
    // The optimistic clear must not discard text typed during the hook's await.
    expect(textarea.value).toBe("first draft, more");
  });

  it("still clears the input on the optimistic path when the draft is unchanged while the hook resolves", async () => {
    let resolveHook!: (p: ChatComposerSendPayload) => void;
    const hooks: AppContext["hooks"] = {
      emitMutate: (async (_name: string, payload: unknown) =>
        new Promise<ChatComposerSendPayload>((resolve) => {
          resolveHook = resolve;
          void payload;
        })) as unknown as AppContext["hooks"]["emitMutate"],
    };
    renderComposer({ hooks });
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    await fireEvent.input(textarea, { target: { value: "unchanged" } });
    await fireEvent.keyDown(textarea, { key: "Enter" });
    resolveHook({ text: "unchanged", cancel: false });
    await waitFor(() => expect(textarea.value).toBe(""));
  });
});

/** A `chat:composer-send`-only fake for `AppContext.hooks` — this test suite's target hook is
 * the only one the composer emits, so a real per-name-keyed `HookBus` would add indirection
 * with no coverage benefit.
 * @param transform Applied to the `ChatComposerSendPayload` the composer emits.
 * @returns The fake `AppContext["hooks"]`.
 * @example
 * ```
 * // internal; used by this suite's own test cases
 * fakeHooks((p) => ({ ...p, cancel: true }));
 * ```
 */
function fakeHooks(transform: (p: ChatComposerSendPayload) => ChatComposerSendPayload): AppContext["hooks"] {
  return {
    emitMutate: (async (_name: string, payload: unknown) => transform(payload as ChatComposerSendPayload)) as unknown as AppContext["hooks"]["emitMutate"],
  };
}

describe("Composer — pre-send interception hook", () => {
  it("sends the text a listener rewrote via chat:composer-send", async () => {
    const hooks = fakeHooks((p) => ({ ...p, text: "REWRITTEN" }));
    const { send } = renderComposer({ hooks });
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    await fireEvent.input(textarea, { target: { value: "original" } });
    await fireEvent.keyDown(textarea, { key: "Enter" });
    expect(send).toHaveBeenCalledWith({ channel: "general", content: "REWRITTEN", audience: publicAudience });
  });

  it("never sends when a listener sets cancel: true", async () => {
    const hooks = fakeHooks((p) => ({ ...p, cancel: true }));
    const { send } = renderComposer({ hooks });
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    await fireEvent.input(textarea, { target: { value: "a system command" } });
    await fireEvent.keyDown(textarea, { key: "Enter" });
    expect(send).not.toHaveBeenCalled();
  });

  it("leaves the input untouched when the send is cancelled by a listener", async () => {
    const hooks = fakeHooks((p) => ({ ...p, cancel: true }));
    renderComposer({ hooks });
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    await fireEvent.input(textarea, { target: { value: "/local-only" } });
    await fireEvent.keyDown(textarea, { key: "Enter" });
    // A cancelled send never reaches ctx.chat.send, so the optimistic clear (which only
    // happens after the hook settles) does not apply here — the module handling the command
    // is responsible for its own UI feedback; the composer neither clears nor shows an error.
    expect(textarea.value).toBe("/local-only");
  });
});

describe("Composer — placeholder", () => {
  it("uses the localized name-placeholder key for a public audience", () => {
    renderComposer();
    expect(screen.getByPlaceholderText("chat.composer.placeholder")).toBeTruthy();
  });

  it("uses the GM placeholder key when audience is gm_only", () => {
    renderComposer({ audience: gmAudience });
    expect(screen.getByPlaceholderText("chat.composer.placeholderGm")).toBeTruthy();
  });
});

describe("Composer — speak-as-actor picker", () => {
  it("defaults to Myself and sends with no actorOwner", async () => {
    const documents = storeWith(ownedActor("act1", "u-self", "Goblin"));
    const { send } = renderComposer({ documents });
    const select = screen.getByLabelText("chat.composer.speakAs") as HTMLSelectElement;
    expect(select.value).toBe("");
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    await fireEvent.input(textarea, { target: { value: "hello" } });
    await fireEvent.keyDown(textarea, { key: "Enter" });
    expect(send).toHaveBeenCalledWith({ channel: "general", content: "hello", audience: publicAudience });
    const sentArg = send.mock.calls[0][0] as Record<string, unknown>;
    expect(sentArg.actorOwner).toBeUndefined();
  });

  it("picking an actor sends actorOwner: {kind: actor, actor_id} exactly", async () => {
    const documents = storeWith(ownedActor("act1", "u-self", "Goblin"));
    const { send } = renderComposer({ documents });
    const select = screen.getByLabelText("chat.composer.speakAs") as HTMLSelectElement;
    await fireEvent.change(select, { target: { value: "act1" } });
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    await fireEvent.input(textarea, { target: { value: "grr" } });
    await fireEvent.keyDown(textarea, { key: "Enter" });
    expect(send).toHaveBeenCalledWith({
      channel: "general",
      content: "grr",
      audience: publicAudience,
      actorOwner: { kind: "actor", actor_id: "act1" },
    });
  });

  it("a player sees only their own actors, not another player's", () => {
    const documents = storeWith(ownedActor("act1", "u-self", "Mine"), ownedActor("act2", "u-other", "TheirsNotMine"));
    renderComposer({ documents, role: "player", selfId: "u-self" });
    const select = screen.getByLabelText("chat.composer.speakAs") as HTMLSelectElement;
    const labels = Array.from(select.options).map((o) => o.textContent);
    expect(labels).toContain("Mine");
    expect(labels).not.toContain("TheirsNotMine");
  });

  it("a GM sees every actor regardless of owner", () => {
    const documents = storeWith(ownedActor("act1", "u-self", "Mine"), ownedActor("act2", "u-other", "TheirsButGmSees"));
    renderComposer({ documents, role: "gm", selfId: "u-self" });
    const select = screen.getByLabelText("chat.composer.speakAs") as HTMLSelectElement;
    const labels = Array.from(select.options).map((o) => o.textContent);
    expect(labels).toContain("Mine");
    expect(labels).toContain("TheirsButGmSees");
  });

  it("reacts to a newly created actor doc appearing in the store", async () => {
    const documents = new DocumentStore();
    renderComposer({ documents, role: "player", selfId: "u-self" });
    const select = screen.getByLabelText("chat.composer.speakAs") as HTMLSelectElement;
    expect(Array.from(select.options).map((o) => o.textContent)).not.toContain("LateJoiner");
    documents.applyCommand(cmd([{ op: "create", doc: ownedActor("act9", "u-self", "LateJoiner") }]));
    await Promise.resolve();
    expect(Array.from(select.options).map((o) => o.textContent)).toContain("LateJoiner");
  });

  it("prunes a dangling selection when the selected actor is deleted", async () => {
    const actor = ownedActor("act1", "u-self", "Goblin");
    const documents = storeWith(actor);
    const { send } = renderComposer({ documents });
    const select = screen.getByLabelText("chat.composer.speakAs") as HTMLSelectElement;
    await fireEvent.change(select, { target: { value: "act1" } });
    expect(select.value).toBe("act1");

    documents.applyCommand(cmd([{ op: "delete", doc: actor }]));
    await Promise.resolve();

    expect(select.value).toBe("");
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    await fireEvent.input(textarea, { target: { value: "hello" } });
    await fireEvent.keyDown(textarea, { key: "Enter" });
    expect(send).toHaveBeenCalledWith({ channel: "general", content: "hello", audience: publicAudience });
    const sentArg = send.mock.calls[0][0] as Record<string, unknown>;
    expect(sentArg.actorOwner).toBeUndefined();
  });

  it("selection is sticky across a send", async () => {
    const documents = storeWith(ownedActor("act1", "u-self", "Goblin"));
    const { send } = renderComposer({ documents });
    const select = screen.getByLabelText("chat.composer.speakAs") as HTMLSelectElement;
    await fireEvent.change(select, { target: { value: "act1" } });
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    await fireEvent.input(textarea, { target: { value: "first" } });
    await fireEvent.keyDown(textarea, { key: "Enter" });
    await fireEvent.input(textarea, { target: { value: "second" } });
    await fireEvent.keyDown(textarea, { key: "Enter" });
    expect(select.value).toBe("act1");
    expect(send).toHaveBeenLastCalledWith({
      channel: "general",
      content: "second",
      audience: publicAudience,
      actorOwner: { kind: "actor", actor_id: "act1" },
    });
  });
});

describe("Composer — @doc link insertion", () => {
  it("opens the doc picker, searches, and inserts a [[doc:id|label]] span at the cursor", async () => {
    const hitDoc = { ...buildActorDoc("w1", "My Doc", { displayName: "My Doc", visual: { kind: "image", asset: "a1" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null, light: null, movement: [], aura: null, sound: null, vfx: null }, "doc1") };
    const searchDocuments: AppContext["searchDocuments"] = vi.fn((_q, _opts, onUpdate) => {
      onUpdate([{ document: hitDoc, score: 1, snippet: "" }]);
      return Promise.resolve({ unsubscribe: () => {} });
    });
    renderComposer({ searchDocuments });
    await fireEvent.click(screen.getByTestId("doc-link-trigger"));
    const search = screen.getByPlaceholderText("chat.composer.docSearchPlaceholder");
    await fireEvent.input(search, { target: { value: "My" } });
    await fireEvent.click(await screen.findByText("My Doc"));
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    expect(textarea.value).toBe("[[doc:doc1|My Doc]]");
    expect(screen.queryByPlaceholderText("chat.composer.docSearchPlaceholder")).toBeNull();
  });

  it("inserts a [[token:id|label]] span for a token-doc_type search hit", async () => {
    const tokenDoc: WireDocument = { ...buildActorDoc("w1", "unused", { displayName: "x", visual: { kind: "image", asset: "a1" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null, light: null, movement: [], aura: null, sound: null, vfx: null }, "tok1"), doc_type: "token", name: "Goblin" };
    const searchDocuments: AppContext["searchDocuments"] = vi.fn((_q, _opts, onUpdate) => {
      onUpdate([{ document: tokenDoc, score: 1, snippet: "" }]);
      return Promise.resolve({ unsubscribe: () => {} });
    });
    renderComposer({ searchDocuments });
    await fireEvent.click(screen.getByTestId("doc-link-trigger"));
    await fireEvent.input(screen.getByPlaceholderText("chat.composer.docSearchPlaceholder"), { target: { value: "Gob" } });
    await fireEvent.click(await screen.findByText("Goblin"));
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    expect(textarea.value).toBe("[[token:tok1|Goblin]]");
  });

  it("a name that strips to empty falls back to the id prefix, never an empty label", async () => {
    // A name made ENTIRELY of grammar-control characters ([, ], |) strips to "" -
    // an empty label is RollError::MalformedDocLink server-side, rejecting the
    // whole message. The inserted span must never carry one.
    const hitDoc = { ...buildActorDoc("w1", "[[||]]", { displayName: "x", visual: { kind: "image", asset: "a1" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null, light: null, movement: [], aura: null, sound: null, vfx: null }, "doc1") };
    const searchDocuments: AppContext["searchDocuments"] = vi.fn((_q, _opts, onUpdate) => {
      onUpdate([{ document: hitDoc, score: 1, snippet: "" }]);
      return Promise.resolve({ unsubscribe: () => {} });
    });
    renderComposer({ searchDocuments });
    await fireEvent.click(screen.getByTestId("doc-link-trigger"));
    await fireEvent.input(screen.getByPlaceholderText("chat.composer.docSearchPlaceholder"), { target: { value: "x" } });
    await fireEvent.click(await screen.findByText("[[||]]"));
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    expect(textarea.value).toBe("[[doc:doc1|doc1]]");
  });

  it("a name containing an unbalanced [ does not desync scan_body's bracket-depth tracking", async () => {
    // An unstripped `[` would increment scan_body's nesting depth without a
    // matching close, causing the span's OWN closing ]] to be silently consumed
    // as an ordinary depth-decrement instead of recognized as the terminator.
    const hitDoc = { ...buildActorDoc("w1", "Foo [Bar", { displayName: "x", visual: { kind: "image", asset: "a1" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null, light: null, movement: [], aura: null, sound: null, vfx: null }, "doc1") };
    const searchDocuments: AppContext["searchDocuments"] = vi.fn((_q, _opts, onUpdate) => {
      onUpdate([{ document: hitDoc, score: 1, snippet: "" }]);
      return Promise.resolve({ unsubscribe: () => {} });
    });
    renderComposer({ searchDocuments });
    await fireEvent.click(screen.getByTestId("doc-link-trigger"));
    await fireEvent.input(screen.getByPlaceholderText("chat.composer.docSearchPlaceholder"), { target: { value: "Foo" } });
    await fireEvent.click(await screen.findByText("Foo [Bar"));
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    expect(textarea.value).toBe("[[doc:doc1|Foo Bar]]");
  });
});

describe("Composer — image insertion", () => {
  it("hides the button when the world's chat-settings images toggle is off", () => {
    renderComposer({ documents: storeWithImagesToggle(false) });
    expect(screen.queryByTestId("image-insert")).toBeNull();
  });

  it("hides the button when no chat-settings doc exists at all (fail-closed default)", () => {
    renderComposer({ documents: new DocumentStore() });
    expect(screen.queryByTestId("image-insert")).toBeNull();
  });

  it("shows the button and inserts a [[asset:id|label]] span at the cursor when the pick resolves", async () => {
    const pickAsset = vi.fn(async () => "00000000-0000-0000-0000-000000000001") as unknown as AppContext["pickAsset"];
    renderComposer({ documents: storeWithImagesToggle(true), pickAsset });
    const btn = screen.getByTestId("image-insert");
    await fireEvent.click(btn);
    expect(pickAsset).toHaveBeenCalledWith({ kind: "image" });
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    expect(textarea.value).toBe("[[asset:00000000-0000-0000-0000-000000000001|00000000]]");
  });

  it("a cancelled pick (null) inserts nothing", async () => {
    const pickAsset = vi.fn(async () => null) as unknown as AppContext["pickAsset"];
    renderComposer({ documents: storeWithImagesToggle(true), pickAsset });
    await fireEvent.click(screen.getByTestId("image-insert"));
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    expect(textarea.value).toBe("");
  });
});

describe("Composer — speak-as-token", () => {
  it("sends a token_instance actor_owner and consumes the pending selection", async () => {
    const speakAsToken = new SpeakAsToken();
    speakAsToken.select("tok1");
    const token = { ...buildActorDoc("w1", "unused", { displayName: "x", visual: { kind: "image", asset: "a1" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null, light: null, movement: [], aura: null, sound: null, vfx: null }, "tok1"), doc_type: "token", name: "Goblin" };
    const documents = new DocumentStore();
    documents.applyCommand({ seq: 1, world_id: "w1", author: "gm", ts: 0, ops: [{ op: "create" as const, doc: token }] });
    const { send } = renderComposer({ documents, speakAsToken });
    // The default test-harness `t` (`over.t ?? ((k) => k)`) returns the raw key without
    // interpolating params, matching this file's other localized-text assertions
    // (e.g. `chat.composer.count`/`chat.composer.placeholder`) — not the interpolated string.
    expect(screen.getByText("chat.composer.speakingAsToken")).toBeTruthy();
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    await fireEvent.input(textarea, { target: { value: "hello" } });
    await fireEvent.keyDown(textarea, { key: "Enter" });
    expect(send).toHaveBeenCalledWith({ channel: "general", content: "hello", audience: publicAudience, actorOwner: { kind: "token_instance", token_id: "tok1" } });
    expect(speakAsToken.tokenId).toBeNull();
  });

  it("shows no indicator when nothing is pending", () => {
    renderComposer();
    expect(screen.queryByText("chat.composer.speakingAsToken")).toBeNull();
  });
});
