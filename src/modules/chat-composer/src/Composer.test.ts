import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { MAX_MESSAGE_CHARS, type WireAudience } from "@shadowcat/core";
import Composer from "./Composer.svelte";

const publicAudience: WireAudience = { kind: "public" };
const gmAudience: WireAudience = { kind: "gm_only" };

function renderComposer(opts: { audience?: WireAudience; send?: (o: unknown) => void } = {}) {
  const send = opts.send ?? vi.fn();
  const context = setAppContextForTest({ chat: { send, edit: vi.fn(), delete: vi.fn() } });
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
