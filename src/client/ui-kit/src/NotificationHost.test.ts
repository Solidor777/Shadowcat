import { test, expect, afterEach, beforeEach, vi } from "vitest";
import { tick } from "svelte";
import { render, screen, fireEvent, cleanup } from "@testing-library/svelte";
import { notifications } from "./notifications.svelte";
import NotificationHost from "./NotificationHost.svelte";

beforeEach(() => {
  // Drain any notifications left by a prior test — `notifications` is a module-level singleton.
  for (const n of [...notifications.items]) notifications.dismiss(n.id);
});
afterEach(() => cleanup());

/**
 * The ✕ dismiss button of the single rendered row, found by its fixed class so the message
 * button (which also dismisses) is never mistaken for it.
 * @returns The ✕ button element.
 * @example await fireEvent.click(dismissButton());
 */
function dismissButton(): HTMLButtonElement {
  const el = document.querySelector<HTMLButtonElement>(".sc-notify-dismiss");
  if (!el) throw new Error("no dismiss button rendered");
  return el;
}

test("renders one entry per active notification, showing its message text", () => {
  notifications.push("warning", "Some targets were skipped.");
  notifications.push("info", "Saved.");
  render(NotificationHost);
  expect(screen.getByText("Some targets were skipped.")).toBeTruthy();
  expect(screen.getByText("Saved.")).toBeTruthy();
});

test("clicking an item's dismiss button calls notifications.dismiss with that item's id", async () => {
  const id = notifications.push("warning", "Some targets were skipped.");
  render(NotificationHost);
  await fireEvent.click(dismissButton());
  expect(notifications.items.find((n) => n.id === id)).toBeUndefined();
  expect(screen.queryByText("Some targets were skipped.")).toBeNull();
});

test("the host container carries aria-live=\"polite\"", () => {
  notifications.push("info", "Saved.");
  const { container } = render(NotificationHost);
  const host = container.querySelector("[aria-live]");
  expect(host?.getAttribute("aria-live")).toBe("polite");
});

test("renders the action button when a notification carries one; clicking runs the action and dismisses the notification", async () => {
  const run = vi.fn();
  const id = notifications.push("warning", "Restore me.", { label: "Reopen windows", run });
  render(NotificationHost);
  const actionBtn = screen.getByRole("button", { name: "Reopen windows" });
  await fireEvent.click(actionBtn);
  expect(run).toHaveBeenCalledTimes(1);
  expect(notifications.items.find((n) => n.id === id)).toBeUndefined();
  expect(screen.queryByText("Restore me.")).toBeNull();
});

test("a notification without an action renders its message button and its dismiss button", () => {
  notifications.push("warning", "Plain.");
  render(NotificationHost);
  expect(screen.getAllByRole("button")).toHaveLength(2);
});

test("an info notification carrying an action is not auto-dismissed", async () => {
  vi.useFakeTimers();
  try {
    const id = notifications.push("info", "Restore available.", {
      label: "Reopen windows",
      run: () => {},
    });
    render(NotificationHost);
    await vi.advanceTimersByTimeAsync(30_000);
    expect(notifications.items.find((n) => n.id === id)).toBeDefined();
    expect(screen.getByRole("button", { name: "Reopen windows" })).toBeTruthy();
  } finally {
    vi.useRealTimers();
  }
});

test("a progress notification is never auto-dismissed, at any level", async () => {
  vi.useFakeTimers();
  try {
    const id = notifications.push("info", "Uploading", undefined, { value: 1, max: 4 });
    render(NotificationHost);
    await vi.advanceTimersByTimeAsync(30_000);
    expect(notifications.items.find((n) => n.id === id)).toBeDefined();
  } finally {
    vi.useRealTimers();
  }
});

test("a determinate progress renders a progressbar with aria-valuenow and aria-valuemax", () => {
  notifications.push("info", "Uploading", undefined, { value: 1, max: 4 });
  render(NotificationHost);
  const bar = screen.getByRole("progressbar");
  expect(bar.getAttribute("aria-valuenow")).toBe("1");
  expect(bar.getAttribute("aria-valuemax")).toBe("4");
  expect(bar.getAttribute("aria-valuemin")).toBe("0");
});

test("an indeterminate progress renders a progressbar with no numeric aria values", () => {
  notifications.push("info", "Working", undefined, { value: null, max: 0 });
  render(NotificationHost);
  const bar = screen.getByRole("progressbar");
  expect(bar.hasAttribute("aria-valuenow")).toBe(false);
  expect(bar.hasAttribute("aria-valuemax")).toBe(false);
  expect(bar.getAttribute("aria-valuetext")).toBeTruthy();
});

test("a notification without progress renders no progressbar", () => {
  notifications.push("info", "Plain.");
  render(NotificationHost);
  expect(screen.queryByRole("progressbar")).toBeNull();
});

test("a live progress update re-renders the bar in place", async () => {
  const id = notifications.push("info", "Uploading", undefined, { value: 1, max: 4 });
  render(NotificationHost);
  notifications.update(id, { progress: { value: 3, max: 4 } });
  await tick();
  expect(screen.getByRole("progressbar").getAttribute("aria-valuenow")).toBe("3");
});

test("clicking a plain notification's message dismisses it", async () => {
  const id = notifications.push("warning", "Click me away.");
  render(NotificationHost);
  await fireEvent.click(screen.getByText("Click me away."));
  expect(notifications.items.find((n) => n.id === id)).toBeUndefined();
});

test("an action button click dismisses exactly once: the row handler never sees the click", async () => {
  const spy = vi.spyOn(notifications, "dismiss");
  const run = vi.fn();
  const id = notifications.push("warning", "Restore me.", { label: "Reopen windows", run });
  render(NotificationHost);
  await fireEvent.click(screen.getByRole("button", { name: "Reopen windows" }));
  expect(run).toHaveBeenCalledTimes(1);
  expect(spy.mock.calls.filter(([dismissed]) => dismissed === id)).toHaveLength(1);
  spy.mockRestore();
});

test("a dismiss button click dismisses exactly once: the row handler never sees the click", async () => {
  const spy = vi.spyOn(notifications, "dismiss");
  const id = notifications.push("warning", "Plain.");
  render(NotificationHost);
  await fireEvent.click(dismissButton());
  expect(spy.mock.calls.filter(([dismissed]) => dismissed === id)).toHaveLength(1);
  spy.mockRestore();
});

test("a message button click dismisses exactly once", async () => {
  const spy = vi.spyOn(notifications, "dismiss");
  const id = notifications.push("warning", "Plain.");
  render(NotificationHost);
  await fireEvent.click(screen.getByText("Plain."));
  expect(spy.mock.calls.filter(([dismissed]) => dismissed === id)).toHaveLength(1);
  spy.mockRestore();
});

test("the message area is a native, focusable button whose accessible name says it dismisses", () => {
  notifications.push("warning", "Plain.");
  render(NotificationHost);
  const messageButton = screen.getByText("Plain.").closest("button") as HTMLButtonElement;
  expect(messageButton).toBeTruthy();
  expect(messageButton.tagName).toBe("BUTTON");
  expect(messageButton.getAttribute("type")).toBe("button");
  expect(messageButton.tabIndex).toBeGreaterThanOrEqual(0);
  messageButton.focus();
  expect(document.activeElement).toBe(messageButton);
  const name = messageButton.getAttribute("aria-label") ?? "";
  expect(name).toContain("Plain.");
  expect(name.toLowerCase()).toContain("dismiss");
  expect(screen.getByRole("button", { name })).toBe(messageButton);
});

test("the message button sits beside the action and dismiss buttons, never nesting them", () => {
  notifications.push("warning", "Restore me.", { label: "Reopen windows", run: () => {} });
  render(NotificationHost);
  const messageButton = screen.getByText("Restore me.").closest("button") as HTMLButtonElement;
  expect(messageButton.querySelector("button")).toBeNull();
  expect(messageButton.contains(screen.getByRole("button", { name: "Reopen windows" }))).toBe(false);
  expect(messageButton.contains(dismissButton())).toBe(false);
});

test("a progress row renders no message button", () => {
  notifications.push("info", "Uploading", undefined, { value: 1, max: 4 });
  render(NotificationHost);
  expect(screen.getByText("Uploading").closest("button")).toBeNull();
});

test("clicking a progress notification's row does not dismiss it", async () => {
  const id = notifications.push("info", "Uploading", undefined, { value: 1, max: 4 });
  render(NotificationHost);
  await fireEvent.click(screen.getByText("Uploading"));
  expect(notifications.items.find((n) => n.id === id)).toBeDefined();
});

test("the dismiss button still dismisses a notification on its own", async () => {
  const id = notifications.push("warning", "Plain.");
  render(NotificationHost);
  await fireEvent.click(dismissButton());
  expect(notifications.items.find((n) => n.id === id)).toBeUndefined();
});

test("a notification pushed AFTER the host mounts renders (the live push path every runtime notice takes)", async () => {
  render(NotificationHost);
  expect(screen.queryByText("It's your turn!")).toBeNull();
  notifications.push("info", "It's your turn!");
  await tick();
  expect(screen.getByText("It's your turn!")).toBeTruthy();
});
