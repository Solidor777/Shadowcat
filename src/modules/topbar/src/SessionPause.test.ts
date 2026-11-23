import { test, expect, afterEach, vi } from "vitest";
import { render, screen, fireEvent, cleanup, waitFor } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { promptDialog, type PromptOptions } from "@shadowcat/ui-kit";
import { DocumentStore, buildWorldSettingsDoc, type KeybindingOptions, type WireOperation } from "@shadowcat/core";
import TopBar from "./TopBar.svelte";
import SessionPauseBanner from "./SessionPauseBanner.svelte";

// The prompt is a shared modal host that needs a mounted DialogHost; the test drives the answer.
vi.mock("@shadowcat/ui-kit", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@shadowcat/ui-kit")>()),
  promptDialog: vi.fn(),
}));
const prompt = vi.mocked(promptDialog);

afterEach(() => cleanup());

/** A store holding a world-settings document with the given session block. */
function storeWith(session: { paused: boolean; message?: string; pauseAudio: boolean } | undefined): DocumentStore {
  const store = new DocumentStore();
  store.applyCommand({
    seq: 1, world_id: "w1", author: "a", ts: 0,
    ops: [{ op: "create", doc: buildWorldSettingsDoc("w1", session ? { session } : {}, "ws1") }],
  });
  return store;
}

test("the banner shows the pause text, and a message with markup stays inert text", () => {
  const store = storeWith({ paused: true, message: "<img src=x onerror=alert(1)>", pauseAudio: false });
  const { container } = render(SessionPauseBanner, { context: setAppContextForTest({ documents: store, store }) });
  const banner = screen.getByTestId("session-pause-banner");
  expect(banner.getAttribute("role")).toBe("status");
  expect(banner.textContent).toContain("topbar.pause.bannerWithMessage");
  expect(container.querySelector("img")).toBeNull();
});

test("the banner shows the plain pause text without a message and is absent when not paused", () => {
  render(SessionPauseBanner, { context: setAppContextForTest({ documents: storeWith({ paused: true, pauseAudio: false }) }) });
  expect(screen.getByTestId("session-pause-banner").textContent).toContain("topbar.pause.banner");
  cleanup();
  render(SessionPauseBanner, { context: setAppContextForTest({ documents: storeWith({ paused: false, pauseAudio: false }) }) });
  expect(screen.queryByTestId("session-pause-banner")).toBeNull();
});

test("a player sees the banner but neither the pause toggle nor the audio switch", () => {
  render(TopBar, { context: setAppContextForTest({ role: "player", documents: storeWith({ paused: true, pauseAudio: false }) }) });
  expect(screen.getByTestId("session-pause-banner")).toBeTruthy();
  expect(screen.queryByTestId("topbar-pause-toggle")).toBeNull();
  expect(screen.queryByTestId("topbar-pause-audio")).toBeNull();
});

test("the GM's resume button writes the session block with the stored block as the pre-image", async () => {
  const sent: WireOperation[][] = [];
  const session = { paused: true, message: "Break", pauseAudio: true };
  render(TopBar, {
    context: setAppContextForTest({ role: "gm", documents: storeWith(session), dispatchIntent: (ops: WireOperation[]) => void sent.push(ops) }),
  });
  await fireEvent.click(screen.getByTestId("topbar-pause-toggle"));
  expect(sent).toHaveLength(1);
  expect(sent[0]).toMatchObject([{ op: "update", doc_id: "ws1", changes: [{ path: "/engine/session", old: session, new: { paused: false, pauseAudio: true } }] }]);
});

test("the GM's pause button asks for a message, then writes it; dismissing the prompt writes nothing", async () => {
  const sent: WireOperation[][] = [];
  render(TopBar, {
    context: setAppContextForTest({ role: "gm", documents: storeWith(undefined), dispatchIntent: (ops: WireOperation[]) => void sent.push(ops) }),
  });
  prompt.mockResolvedValueOnce("  Back in five  ");
  await fireEvent.click(screen.getByTestId("topbar-pause-toggle"));
  await waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]).toMatchObject([{ changes: [{ path: "/engine/session", old: null, new: { paused: true, pauseAudio: false, message: "Back in five" } }] }]);

  prompt.mockResolvedValueOnce(null);
  await fireEvent.click(screen.getByTestId("topbar-pause-toggle"));
  await waitFor(() => expect(prompt).toHaveBeenCalledTimes(2));
  await Promise.resolve();
  expect(sent).toHaveLength(1);

  // The validator the prompt receives enforces the server's limits.
  const opts = prompt.mock.calls[0][1] as PromptOptions;
  expect(opts.validate!("ok")).toBeNull();
  expect(opts.validate!("x".repeat(281))).toBe("topbar.pause.messageTooLong");
  expect(opts.validate!("a\nb")).toBe("topbar.pause.messageInvalid");
});

test("the audio switch writes pauseAudio and keeps the pause state", async () => {
  const sent: WireOperation[][] = [];
  render(TopBar, {
    context: setAppContextForTest({ role: "gm", documents: storeWith({ paused: true, message: "Break", pauseAudio: false }), dispatchIntent: (ops: WireOperation[]) => void sent.push(ops) }),
  });
  await fireEvent.click(screen.getByTestId("topbar-pause-audio"));
  expect(sent[0]).toMatchObject([{ changes: [{ new: { paused: true, pauseAudio: true, message: "Break" } }] }]);
});

test("the GM registers the pause keybinding on mount and disposes it on unmount; a player registers none", () => {
  const dispose = vi.fn();
  const register = vi.fn((_id: string, _opts: KeybindingOptions) => ({ dispose, conflictsWith: null }));
  const { unmount } = render(TopBar, { props: { registerKeybinding: register }, context: setAppContextForTest({ role: "gm", documents: storeWith(undefined) }) });
  expect(register).toHaveBeenCalledTimes(1);
  expect(register.mock.calls[0][0]).toBe("toggle-pause");
  expect(register.mock.calls[0][1].defaultCombo).toBe("Alt+KeyP");
  unmount();
  expect(dispose).toHaveBeenCalledTimes(1);
  cleanup();
  const playerRegister = vi.fn((_id: string, _opts: KeybindingOptions) => ({ dispose, conflictsWith: null }));
  render(TopBar, { props: { registerKeybinding: playerRegister }, context: setAppContextForTest({ role: "player", documents: storeWith(undefined) }) });
  expect(playerRegister).not.toHaveBeenCalled();
});
