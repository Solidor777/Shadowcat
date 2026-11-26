import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import DialogHost from "./DialogHost.svelte";
import { confirmDialog, type ConfirmOptions } from "./confirmDialog";
import { dialogQueue } from "./dialogQueue.svelte";

const ctx = { t: (key: string, params?: Record<string, string | number>) => (params ? `${key}:${JSON.stringify(params)}` : key) };
const opts: ConfirmOptions = {
  titleKey: "x.title",
  messageKey: "x.message",
  messageParams: { name: "n" },
  confirmLabelKey: "x.ok",
  cancelLabelKey: "x.cancel",
  danger: true,
};

/** Resolves once `promise` has settled, returning its value, or `"pending"` after a macrotask. */
async function settled<T>(promise: Promise<T>): Promise<T | "pending"> {
  return Promise.race([promise, new Promise<"pending">((r) => setTimeout(() => r("pending"), 20))]);
}

describe("confirmDialog", () => {
  afterEach(() => {
    dialogQueue.items = [];
  });

  it("renders the translated title, message and labels as an alert dialog when danger", async () => {
    render(DialogHost);
    void confirmDialog(ctx, opts);
    const dialog = await screen.findByTestId("confirm-dialog");
    expect(dialog.getAttribute("role")).toBe("alertdialog");
    expect(screen.getByText("x.title")).toBeTruthy();
    expect(screen.getByText('x.message:{"name":"n"}')).toBeTruthy();
    expect(screen.getByText("x.ok")).toBeTruthy();
    expect(screen.getByText("x.cancel")).toBeTruthy();
  });

  it("focuses the primary button on mount", async () => {
    render(DialogHost);
    void confirmDialog(ctx, opts);
    const confirm = await screen.findByTestId("confirm-dialog-confirm");
    await vi.waitFor(() => expect(document.activeElement).toBe(confirm));
  });

  it("the confirm button resolves true and removes the dialog", async () => {
    render(DialogHost);
    const result = confirmDialog(ctx, opts);
    await fireEvent.click(await screen.findByTestId("confirm-dialog-confirm"));
    expect(await result).toBe(true);
    expect(screen.queryByTestId("confirm-dialog")).toBeNull();
  });

  it("Enter on the dialog confirms", async () => {
    render(DialogHost);
    const result = confirmDialog(ctx, opts);
    const dialog = await screen.findByTestId("confirm-dialog");
    await fireEvent.keyDown(dialog, { key: "Enter" });
    expect(await result).toBe(true);
  });

  it("Enter on the Cancel button does not confirm", async () => {
    render(DialogHost);
    const result = confirmDialog(ctx, opts);
    await fireEvent.keyDown(await screen.findByTestId("confirm-dialog-cancel"), { key: "Enter" });
    expect(await settled(result)).toBe("pending");
  });

  it("the Cancel button resolves false", async () => {
    render(DialogHost);
    const result = confirmDialog(ctx, opts);
    await fireEvent.click(await screen.findByTestId("confirm-dialog-cancel"));
    expect(await result).toBe(false);
  });

  it("Escape resolves false through the coordinator", async () => {
    render(DialogHost);
    const result = confirmDialog(ctx, opts);
    await screen.findByTestId("confirm-dialog");
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
    expect(await result).toBe(false);
    expect(screen.queryByTestId("confirm-dialog")).toBeNull();
  });

  it("a scrim click does not dismiss", async () => {
    const { container } = render(DialogHost);
    const result = confirmDialog(ctx, opts);
    await screen.findByTestId("confirm-dialog");
    await fireEvent.click(container.querySelector(".sc-dialog-scrim") as HTMLElement);
    expect(await settled(result)).toBe("pending");
    expect(screen.queryByTestId("confirm-dialog")).not.toBeNull();
  });

  it("Tab from the last button wraps to the first", async () => {
    render(DialogHost);
    void confirmDialog(ctx, opts);
    const cancel = await screen.findByTestId("confirm-dialog-cancel");
    const confirm = screen.getByTestId("confirm-dialog-confirm");
    confirm.focus();
    await fireEvent.keyDown(confirm, { key: "Tab" });
    expect(document.activeElement).toBe(cancel);
  });
});
