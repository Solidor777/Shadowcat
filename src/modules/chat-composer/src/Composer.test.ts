import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import type { AppContext } from "@shadowcat/ui-kit";
import { SpeakAsToken } from "@shadowcat/ui-kit";
import { DocumentStore, buildActorDoc, MAX_MESSAGE_CHARS, type WireAudience, type WireCommand, type WireDocument } from "@shadowcat/core";
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
  return { ...buildActorDoc("w1", name, { displayName: name, visual: { kind: "image", asset: "a1" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null }, id), owner };
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
  } = {},
) {
  const send = opts.send ?? vi.fn<(o: unknown) => Promise<void>>(async () => {});
  const context = setAppContextForTest({
    chat: { send, edit: vi.fn(async () => {}), delete: vi.fn(async () => {}), recalc: vi.fn(async () => {}) },
    documents: opts.documents ?? new DocumentStore(),
    role: opts.role ?? "player",
    selfId: opts.selfId ?? "u-self",
    searchDocuments: opts.searchDocuments,
    speakAsToken: opts.speakAsToken,
  });
  render(Composer, { props: { channel: "general", audience: opts.audience ?? publicAudience, placeholderName: "Alice" }, context });
  return { send };
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
    // Input still clears optimistically; the rejection is what surfaces.
    expect(textarea.value).toBe("");
    expect(await screen.findByText(/Rate limit exceeded/i)).toBeTruthy();
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
    const hitDoc = { ...buildActorDoc("w1", "My Doc", { displayName: "My Doc", visual: { kind: "image", asset: "a1" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null }, "doc1") };
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
    const tokenDoc: WireDocument = { ...buildActorDoc("w1", "unused", { displayName: "x", visual: { kind: "image", asset: "a1" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null }, "tok1"), doc_type: "token", name: "Goblin" };
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
    const hitDoc = { ...buildActorDoc("w1", "[[||]]", { displayName: "x", visual: { kind: "image", asset: "a1" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null }, "doc1") };
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
    const hitDoc = { ...buildActorDoc("w1", "Foo [Bar", { displayName: "x", visual: { kind: "image", asset: "a1" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null }, "doc1") };
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

describe("Composer — speak-as-token", () => {
  it("sends a token_instance actor_owner and consumes the pending selection", async () => {
    const speakAsToken = new SpeakAsToken();
    speakAsToken.select("tok1");
    const token = { ...buildActorDoc("w1", "unused", { displayName: "x", visual: { kind: "image", asset: "a1" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false, vision: null }, "tok1"), doc_type: "token", name: "Goblin" };
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
