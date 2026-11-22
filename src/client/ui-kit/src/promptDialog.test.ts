import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import DialogHost from "./DialogHost.svelte";
import { promptDialog, type PromptOptions } from "./promptDialog";
import { dialogQueue } from "./dialogQueue.svelte";

const ctx = { t: (key: string) => key };
const opts: PromptOptions = {
  titleKey: "x.title",
  messageKey: "x.message",
  confirmLabelKey: "x.ok",
  cancelLabelKey: "x.cancel",
  initialValue: "start",
  placeholder: "type here",
};

/** Resolves once `promise` has settled, returning its value, or `"pending"` after a macrotask. */
async function settled<T>(promise: Promise<T>): Promise<T | "pending"> {
  return Promise.race([promise, new Promise<"pending">((r) => setTimeout(() => r("pending"), 20))]);
}

/** The prompt's text field. */
function input(): HTMLInputElement {
  return screen.getByTestId("prompt-dialog-input") as HTMLInputElement;
}

describe("promptDialog", () => {
  afterEach(() => {
    dialogQueue.items = [];
  });

  it("focuses the field with its initial value selected", async () => {
    render(DialogHost);
    void promptDialog(ctx, opts);
    await screen.findByTestId("prompt-dialog");
    await vi.waitFor(() => expect(document.activeElement).toBe(input()));
    expect(input().value).toBe("start");
    expect(input().selectionStart).toBe(0);
    expect(input().selectionEnd).toBe(5);
    expect(input().placeholder).toBe("type here");
  });

  it("Enter submits the entered text", async () => {
    render(DialogHost);
    const result = promptDialog(ctx, opts);
    await screen.findByTestId("prompt-dialog");
    await fireEvent.input(input(), { target: { value: "typed" } });
    await fireEvent.keyDown(input(), { key: "Enter" });
    expect(await result).toBe("typed");
    expect(screen.queryByTestId("prompt-dialog")).toBeNull();
  });

  it("the confirm button submits the entered text", async () => {
    render(DialogHost);
    const result = promptDialog(ctx, opts);
    await screen.findByTestId("prompt-dialog");
    await fireEvent.input(input(), { target: { value: "clicked" } });
    await fireEvent.click(screen.getByTestId("prompt-dialog-confirm"));
    expect(await result).toBe("clicked");
  });

  it("a validation failure shows the error, disables confirm and makes Enter a no-op", async () => {
    render(DialogHost);
    const result = promptDialog(ctx, { ...opts, validate: (v) => (v === "" ? "must not be empty" : null) });
    await screen.findByTestId("prompt-dialog");
    await fireEvent.input(input(), { target: { value: "" } });
    expect(screen.getByTestId("prompt-dialog-error").textContent).toBe("must not be empty");
    expect((screen.getByTestId("prompt-dialog-confirm") as HTMLButtonElement).disabled).toBe(true);
    await fireEvent.keyDown(input(), { key: "Enter" });
    expect(await settled(result)).toBe("pending");
    await fireEvent.input(input(), { target: { value: "ok" } });
    await fireEvent.keyDown(input(), { key: "Enter" });
    expect(await result).toBe("ok");
  });

  it("the Cancel button resolves null", async () => {
    render(DialogHost);
    const result = promptDialog(ctx, opts);
    await fireEvent.click(await screen.findByTestId("prompt-dialog-cancel"));
    expect(await result).toBeNull();
  });

  it("Escape resolves null through the coordinator", async () => {
    render(DialogHost);
    const result = promptDialog(ctx, opts);
    await screen.findByTestId("prompt-dialog");
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
    expect(await result).toBeNull();
  });
});
