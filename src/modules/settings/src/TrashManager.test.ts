import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { confirmDialog } from "@shadowcat/ui-kit";
import TrashManager from "./TrashManager.svelte";

vi.mock("@shadowcat/ui-kit", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@shadowcat/ui-kit")>();
  return { ...actual, confirmDialog: vi.fn() };
});

vi.mock("@shadowcat/core", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@shadowcat/core")>();
  return {
    ...actual,
    listTrash: vi.fn(),
    emptyTrash: vi.fn(),
    deleteTrashEntry: vi.fn(),
  };
});

/** Two entries, newest first. */
const ENTRIES = [
  {
    id: "2000-00000000-0000-0000-0000-000000000002",
    original_path: "/data/assets/b.png",
    kind: "file" as const,
    trashed_at_ms: 2000,
    size_bytes: 2048,
  },
  {
    id: "1000-00000000-0000-0000-0000-000000000001",
    original_path: "/data/assets/world",
    kind: "dir" as const,
    trashed_at_ms: 1000,
    size_bytes: 5,
  },
];

describe("TrashManager", () => {
  beforeEach(async () => {
    vi.clearAllMocks();
    const core = await import("@shadowcat/core");
    vi.mocked(core.listTrash).mockResolvedValue(ENTRIES);
    vi.mocked(core.emptyTrash).mockResolvedValue({ entries: 2, bytes: 2053 });
    vi.mocked(core.deleteTrashEntry).mockResolvedValue(undefined);
  });

  it("renders nothing and fetches nothing for a non-admin, even a world GM", async () => {
    const { listTrash } = await import("@shadowcat/core");
    const { container } = render(TrashManager, {
      context: setAppContextForTest({ role: "gm", serverRole: "user" }),
    });
    expect(container.querySelector(".trash-manager")).toBeNull();
    expect(vi.mocked(listTrash)).not.toHaveBeenCalled();
  });

  it("lists each entry with its size through the shared byte formatter", async () => {
    render(TrashManager, { context: setAppContextForTest({ serverRole: "admin" }) });
    expect(await screen.findByText("/data/assets/b.png")).toBeTruthy();
    expect(screen.getByText("/data/assets/world")).toBeTruthy();
    // 2048 bytes renders as "2.0 KiB", not the raw count.
    expect(screen.getByText(/2\.0 KiB/)).toBeTruthy();
    expect(screen.getByText(/5 B/)).toBeTruthy();
  });

  it("shows the empty state when the trash holds nothing", async () => {
    const { listTrash } = await import("@shadowcat/core");
    vi.mocked(listTrash).mockResolvedValue([]);
    render(TrashManager, { context: setAppContextForTest({ serverRole: "admin" }) });
    expect(await screen.findByText("settings.trash.empty")).toBeTruthy();
  });

  it("delete-one asks for confirmation, deletes that entry, and reloads", async () => {
    const { listTrash, deleteTrashEntry } = await import("@shadowcat/core");
    vi.mocked(confirmDialog).mockResolvedValue(true);
    render(TrashManager, { context: setAppContextForTest({ serverRole: "admin" }) });
    const buttons = await screen.findAllByRole("button", { name: "settings.trash.deleteOne" });
    await fireEvent.click(buttons[1]);
    expect(confirmDialog).toHaveBeenCalledWith(
      expect.anything(),
      expect.objectContaining({
        messageKey: "settings.trash.deleteConfirm",
        messageParams: { path: "/data/assets/world" },
        danger: true,
      }),
    );
    await vi.waitFor(() => expect(vi.mocked(deleteTrashEntry)).toHaveBeenCalledWith(ENTRIES[1].id));
    await vi.waitFor(() => expect(vi.mocked(listTrash)).toHaveBeenCalledTimes(2));
  });

  it("a declined confirm deletes and empties nothing", async () => {
    const { deleteTrashEntry, emptyTrash } = await import("@shadowcat/core");
    vi.mocked(confirmDialog).mockResolvedValue(false);
    render(TrashManager, { context: setAppContextForTest({ serverRole: "admin" }) });
    const del = await screen.findAllByRole("button", { name: "settings.trash.deleteOne" });
    await fireEvent.click(del[0]);
    await fireEvent.click(screen.getByRole("button", { name: "settings.trash.emptyAll" }));
    await vi.waitFor(() => expect(confirmDialog).toHaveBeenCalledTimes(2));
    await new Promise((r) => setTimeout(r, 0));
    expect(vi.mocked(deleteTrashEntry)).not.toHaveBeenCalled();
    expect(vi.mocked(emptyTrash)).not.toHaveBeenCalled();
  });

  it("an already-gone entry refreshes silently instead of showing an error", async () => {
    const { listTrash, deleteTrashEntry, TrashEntryGoneError } = await import("@shadowcat/core");
    vi.mocked(confirmDialog).mockResolvedValue(true);
    vi.mocked(deleteTrashEntry).mockRejectedValueOnce(new TrashEntryGoneError("not found"));
    render(TrashManager, { context: setAppContextForTest({ serverRole: "admin" }) });
    const buttons = await screen.findAllByRole("button", { name: "settings.trash.deleteOne" });
    await fireEvent.click(buttons[0]);
    await vi.waitFor(() => expect(vi.mocked(listTrash)).toHaveBeenCalledTimes(2));
    expect(screen.queryByText("settings.trash.deleteError")).toBeNull();
  });

  it("any other delete failure shows the delete error", async () => {
    const { deleteTrashEntry } = await import("@shadowcat/core");
    vi.mocked(confirmDialog).mockResolvedValue(true);
    vi.mocked(deleteTrashEntry).mockRejectedValueOnce(new Error("disk on fire"));
    render(TrashManager, { context: setAppContextForTest({ serverRole: "admin" }) });
    const buttons = await screen.findAllByRole("button", { name: "settings.trash.deleteOne" });
    await fireEvent.click(buttons[0]);
    expect(await screen.findByText("settings.trash.deleteError")).toBeTruthy();
  });

  it("empty-all asks for confirmation, empties, reports the totals and reloads", async () => {
    const { listTrash, emptyTrash } = await import("@shadowcat/core");
    vi.mocked(confirmDialog).mockResolvedValue(true);
    render(TrashManager, {
      context: setAppContextForTest({
        serverRole: "admin",
        plural: (key, count, params) => `${key}|${count}|${params?.bytes}`,
      }),
    });
    await fireEvent.click(await screen.findByRole("button", { name: "settings.trash.emptyAll" }));
    expect(confirmDialog).toHaveBeenCalledWith(
      expect.anything(),
      expect.objectContaining({
        messageKey: "settings.trash.emptyConfirm",
        messageParams: { n: 2 },
        danger: true,
      }),
    );
    await vi.waitFor(() => expect(vi.mocked(emptyTrash)).toHaveBeenCalledTimes(1));
    await vi.waitFor(() => expect(vi.mocked(listTrash)).toHaveBeenCalledTimes(2));
    // 2053 bytes renders through the shared formatter as "2.0 KiB".
    expect(await screen.findByText("settings.trash.emptyResult|2|2.0 KiB")).toBeTruthy();
  });

  it("empty-all states the count as a lower bound while more pages exist", async () => {
    const { listTrash } = await import("@shadowcat/core");
    vi.mocked(confirmDialog).mockResolvedValue(false);
    const full = Array.from({ length: 200 }, (_, i) => ({
      id: `${5000 - i}-00000000-0000-0000-0000-${String(i).padStart(12, "0")}`,
      original_path: `/data/f${i}`,
      kind: "file" as const,
      trashed_at_ms: 5000 - i,
      size_bytes: 1,
    }));
    vi.mocked(listTrash).mockResolvedValueOnce(full);
    render(TrashManager, { context: setAppContextForTest({ serverRole: "admin" }) });
    await fireEvent.click(await screen.findByRole("button", { name: "settings.trash.emptyAll" }));
    expect(confirmDialog).toHaveBeenCalledWith(
      expect.anything(),
      expect.objectContaining({
        messageKey: "settings.trash.emptyConfirmMore",
        messageParams: { n: 200 },
      }),
    );
  });

  it("a full page offers Load more, which fetches the page after the last entry", async () => {
    const { listTrash } = await import("@shadowcat/core");
    const full = Array.from({ length: 200 }, (_, i) => ({
      id: `${5000 - i}-00000000-0000-0000-0000-${String(i).padStart(12, "0")}`,
      original_path: `/data/f${i}`,
      kind: "file" as const,
      trashed_at_ms: 5000 - i,
      size_bytes: 1,
    }));
    vi.mocked(listTrash).mockResolvedValueOnce(full).mockResolvedValueOnce([]);
    render(TrashManager, { context: setAppContextForTest({ serverRole: "admin" }) });
    await fireEvent.click(await screen.findByRole("button", { name: "settings.trash.loadMore" }));
    await vi.waitFor(() =>
      expect(vi.mocked(listTrash)).toHaveBeenLastCalledWith({
        limit: 200,
        before: full[199].id,
      }),
    );
    await vi.waitFor(() =>
      expect(screen.queryByRole("button", { name: "settings.trash.loadMore" })).toBeNull(),
    );
  });
});
