import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { confirmDialog } from "@shadowcat/ui-kit";
import BackupManager from "./BackupManager.svelte";

vi.mock("@shadowcat/ui-kit", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@shadowcat/ui-kit")>();
  return { ...actual, confirmDialog: vi.fn() };
});

/** One list row in the shape `listBackups` returns (numbers, not bigints). */
function entry(id: string, at: number, kind = "manual") {
  return {
    id,
    manifest: {
      shadowcat_version: "0.1.0",
      created_at_unix_ms: at,
      source_db: "shadowcat.db",
      source_assets_dir: "assets",
      asset_file_count: 2,
      db_bytes: 2048,
      kind,
      note: null,
    },
    total_bytes: 4096,
  };
}

/** A listing with two backups, no failed restore and no retention limit. */
function listing(over: Record<string, unknown> = {}) {
  return {
    entries: [entry("backup-2", 2_000), entry("backup-1", 1_000, "pre-restore")],
    policy: { keep_last: null, keep_days: null },
    root: "backups",
    restore_failed: null,
    ...over,
  };
}

vi.mock("@shadowcat/core", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@shadowcat/core")>();
  return {
    ...actual,
    adminBackup: vi.fn(),
    listBackups: vi.fn(),
    deleteBackup: vi.fn(),
    restoreBackup: vi.fn(),
    pruneBackups: vi.fn(),
  };
});

describe("BackupManager", () => {
  beforeEach(async () => {
    vi.clearAllMocks();
    const core = await import("@shadowcat/core");
    vi.mocked(core.listBackups).mockResolvedValue(listing() as never);
    vi.mocked(core.adminBackup).mockResolvedValue({ db_bytes: 2048 } as never);
    vi.mocked(core.deleteBackup).mockResolvedValue(undefined);
    vi.mocked(core.restoreBackup).mockResolvedValue({ staged: true, pre_restore_id: "backup-9" });
    vi.mocked(core.pruneBackups).mockResolvedValue({ removed: ["backup-1"] });
  });

  it("renders nothing and fetches nothing for a non-admin, even a world GM", async () => {
    const { listBackups } = await import("@shadowcat/core");
    const { container } = render(BackupManager, {
      context: setAppContextForTest({ role: "gm", serverRole: "user" }),
    });
    expect(container.querySelector(".backup-manager")).toBeNull();
    expect(vi.mocked(listBackups)).not.toHaveBeenCalled();
  });

  it("lists every backup with its kind, size, asset count and version", async () => {
    render(BackupManager, { context: setAppContextForTest({ serverRole: "admin" }) });
    const rows = await screen.findAllByRole("listitem");
    expect(rows).toHaveLength(2);
    expect(rows[0].textContent).toContain("settings.backups.kind.manual");
    expect(rows[1].textContent).toContain("settings.backups.kind.pre-restore");
    expect(rows[0].textContent).toContain("4.0 KiB");
    expect(rows[0].textContent).toContain("settings.backups.assetFiles");
    expect(rows[0].textContent).toContain("settings.backups.version");
    expect(screen.getByText("settings.backups.root")).toBeTruthy();
  });

  it("shows an empty state when there are no backups", async () => {
    const { listBackups } = await import("@shadowcat/core");
    vi.mocked(listBackups).mockResolvedValue(listing({ entries: [] }) as never);
    render(BackupManager, { context: setAppContextForTest({ serverRole: "admin" }) });
    expect(await screen.findByText("settings.backups.empty")).toBeTruthy();
  });

  it("reports a failed list load and keeps the section", async () => {
    const { listBackups } = await import("@shadowcat/core");
    vi.mocked(listBackups).mockRejectedValue(new Error("forbidden"));
    render(BackupManager, { context: setAppContextForTest({ serverRole: "admin" }) });
    expect(await screen.findByText("settings.backups.error")).toBeTruthy();
    expect(screen.getByText("settings.backups.backupNow")).toBeTruthy();
  });

  describe("Back up now", () => {
    /** A recording `notifyProgress` seam. */
    function seam() {
      const events: string[] = [];
      const notifyProgress = vi.fn((message: string, progress: { value: number | null }) => {
        events.push(`push ${message} ${progress.value}`);
        return { id: "p", update: () => {}, dismiss: () => events.push("dismiss") };
      });
      return { events, notifyProgress };
    }

    it("shows an indeterminate progress notification while the backup runs, then dismisses it, reports the size and reloads", async () => {
      const { adminBackup, listBackups } = await import("@shadowcat/core");
      let release: (v: unknown) => void = () => {};
      vi.mocked(adminBackup).mockImplementationOnce(
        () => new Promise((resolve) => (release = resolve)) as never,
      );
      const { events, notifyProgress } = seam();
      const notify = vi.fn();
      render(BackupManager, {
        context: setAppContextForTest({ serverRole: "admin", notifyProgress, notify }),
      });
      await screen.findAllByRole("listitem");
      await fireEvent.click(screen.getByText("settings.backups.backupNow"));
      await vi.waitFor(() => expect(events).toEqual(["push settings.backups.running null"]));
      expect((screen.getByText("settings.backups.backupNow") as HTMLButtonElement).disabled).toBe(true);
      release({ db_bytes: 2048 });
      await vi.waitFor(() => expect(events).toContain("dismiss"));
      expect(notify).toHaveBeenCalledWith("settings.backups.done", "info");
      await vi.waitFor(() => expect(vi.mocked(listBackups)).toHaveBeenCalledTimes(2));
      expect((screen.getByText("settings.backups.backupNow") as HTMLButtonElement).disabled).toBe(false);
    });

    it("a rejected backup still dismisses the progress notification and reports the failure", async () => {
      const { adminBackup } = await import("@shadowcat/core");
      vi.mocked(adminBackup).mockRejectedValueOnce(new Error("disk full"));
      const { events, notifyProgress } = seam();
      const notify = vi.fn();
      render(BackupManager, {
        context: setAppContextForTest({ serverRole: "admin", notifyProgress, notify }),
      });
      await screen.findAllByRole("listitem");
      await fireEvent.click(screen.getByText("settings.backups.backupNow"));
      await vi.waitFor(() => expect(events).toContain("dismiss"));
      expect(notify).toHaveBeenCalledWith("settings.backups.backupError", "error");
    });
  });

  describe("Restore", () => {
    it("asks for confirmation first, naming the server stop, and a declined dialog restores nothing", async () => {
      const { restoreBackup } = await import("@shadowcat/core");
      vi.mocked(confirmDialog).mockResolvedValueOnce(false);
      render(BackupManager, { context: setAppContextForTest({ serverRole: "admin" }) });
      const buttons = await screen.findAllByRole("button", { name: "settings.backups.restoreRow" });
      await fireEvent.click(buttons[0]);
      await vi.waitFor(() => expect(confirmDialog).toHaveBeenCalledTimes(1));
      expect(confirmDialog).toHaveBeenCalledWith(
        expect.anything(),
        expect.objectContaining({
          messageKey: "settings.backups.restoreConfirm",
          confirmLabelKey: "settings.backups.restoreAndStop",
          danger: true,
        }),
      );
      // Let the resolved `false` reach the handler before asserting nothing was restored.
      await new Promise((r) => setTimeout(r, 0));
      expect(vi.mocked(restoreBackup)).not.toHaveBeenCalled();
      expect(screen.queryByRole("status")).toBeNull();
    });

    it("a confirmed restore stages it, tells the admin to start the server again and locks the actions", async () => {
      const { restoreBackup } = await import("@shadowcat/core");
      vi.mocked(confirmDialog).mockResolvedValue(true);
      const notify = vi.fn();
      render(BackupManager, { context: setAppContextForTest({ serverRole: "admin", notify }) });
      const buttons = await screen.findAllByRole("button", { name: "settings.backups.restoreRow" });
      await fireEvent.click(buttons[1]);
      await vi.waitFor(() => expect(vi.mocked(restoreBackup)).toHaveBeenCalledWith("backup-1"));
      expect((await screen.findByRole("status")).textContent).toContain("settings.backups.stopping");
      expect(notify).toHaveBeenCalledWith("settings.backups.restoreStaged", "info");
      for (const b of screen.getAllByRole("button", { name: "settings.backups.restoreRow" })) {
        expect((b as HTMLButtonElement).disabled).toBe(true);
      }
      expect((screen.getByText("settings.backups.backupNow") as HTMLButtonElement).disabled).toBe(true);
    });

    it("a rejected restore shows its error and leaves the actions usable", async () => {
      const { restoreBackup } = await import("@shadowcat/core");
      vi.mocked(confirmDialog).mockResolvedValue(true);
      vi.mocked(restoreBackup).mockRejectedValueOnce(new Error("not found"));
      render(BackupManager, { context: setAppContextForTest({ serverRole: "admin" }) });
      const buttons = await screen.findAllByRole("button", { name: "settings.backups.restoreRow" });
      await fireEvent.click(buttons[0]);
      expect(await screen.findByText("settings.backups.restoreError")).toBeTruthy();
      expect(screen.queryByRole("status")).toBeNull();
      expect((buttons[0] as HTMLButtonElement).disabled).toBe(false);
    });
  });

  describe("Delete", () => {
    it("asks for confirmation, deletes the row's backup and reloads; a declined dialog deletes nothing", async () => {
      const { deleteBackup, listBackups } = await import("@shadowcat/core");
      vi.mocked(confirmDialog).mockResolvedValueOnce(false);
      render(BackupManager, { context: setAppContextForTest({ serverRole: "admin" }) });
      let buttons = await screen.findAllByRole("button", { name: "settings.backups.deleteRow" });
      await fireEvent.click(buttons[0]);
      await vi.waitFor(() => expect(confirmDialog).toHaveBeenCalledTimes(1));
      await new Promise((r) => setTimeout(r, 0));
      expect(vi.mocked(deleteBackup)).not.toHaveBeenCalled();

      vi.mocked(confirmDialog).mockResolvedValue(true);
      buttons = await screen.findAllByRole("button", { name: "settings.backups.deleteRow" });
      await fireEvent.click(buttons[1]);
      await vi.waitFor(() => expect(vi.mocked(deleteBackup)).toHaveBeenCalledWith("backup-1"));
      await vi.waitFor(() => expect(vi.mocked(listBackups)).toHaveBeenCalledTimes(2));
    });

    it("a rejected delete shows its error", async () => {
      const { deleteBackup } = await import("@shadowcat/core");
      vi.mocked(confirmDialog).mockResolvedValue(true);
      vi.mocked(deleteBackup).mockRejectedValueOnce(new Error("nope"));
      render(BackupManager, { context: setAppContextForTest({ serverRole: "admin" }) });
      const buttons = await screen.findAllByRole("button", { name: "settings.backups.deleteRow" });
      await fireEvent.click(buttons[0]);
      expect(await screen.findByText("settings.backups.deleteError")).toBeTruthy();
    });
  });

  describe("retention policy", () => {
    it("says every backup is kept and offers no Apply now when no limit is set", async () => {
      render(BackupManager, { context: setAppContextForTest({ serverRole: "admin" }) });
      expect(await screen.findByText("settings.backups.policyKeepAll")).toBeTruthy();
      expect((screen.getByText("settings.backups.applyNow") as HTMLButtonElement).disabled).toBe(true);
    });

    it("shows the configured limits read-only and Apply now prunes, reports the count and reloads", async () => {
      const { pruneBackups, listBackups } = await import("@shadowcat/core");
      vi.mocked(listBackups).mockResolvedValue(
        listing({ policy: { keep_last: 1, keep_days: 7 } }) as never,
      );
      render(BackupManager, { context: setAppContextForTest({ serverRole: "admin" }) });
      await screen.findByText("settings.backups.applyNow");
      expect(screen.getByText(/settings\.backups\.policyLast/)).toBeTruthy();
      expect(screen.getByText(/settings\.backups\.policyDays/)).toBeTruthy();
      expect(screen.queryByRole("textbox")).toBeNull();
      const apply = screen.getByText("settings.backups.applyNow") as HTMLButtonElement;
      expect(apply.disabled).toBe(false);
      await fireEvent.click(apply);
      await vi.waitFor(() => expect(vi.mocked(pruneBackups)).toHaveBeenCalledTimes(1));
      expect(await screen.findByText("settings.backups.pruned")).toBeTruthy();
      await vi.waitFor(() => expect(vi.mocked(listBackups)).toHaveBeenCalledTimes(2));
    });

    it("reports a prune that removed nothing, and a failed prune", async () => {
      const { pruneBackups, listBackups } = await import("@shadowcat/core");
      vi.mocked(listBackups).mockResolvedValue(
        listing({ policy: { keep_last: 5, keep_days: null } }) as never,
      );
      vi.mocked(pruneBackups).mockResolvedValueOnce({ removed: [] });
      render(BackupManager, { context: setAppContextForTest({ serverRole: "admin" }) });
      await fireEvent.click(await screen.findByText("settings.backups.applyNow"));
      expect(await screen.findByText("settings.backups.prunedNone")).toBeTruthy();
      vi.mocked(pruneBackups).mockRejectedValueOnce(new Error("busy"));
      await fireEvent.click(screen.getByText("settings.backups.applyNow"));
      expect(await screen.findByText("settings.backups.pruneError")).toBeTruthy();
    });
  });

  describe("failed restore banner", () => {
    it("is absent when the server reports no failed restore", async () => {
      const { container } = render(BackupManager, {
        context: setAppContextForTest({ serverRole: "admin" }),
      });
      await screen.findAllByRole("listitem");
      expect(container.querySelector(".restore-failed")).toBeNull();
    });

    it("names the requested backup and the error when the server reports one", async () => {
      const { listBackups } = await import("@shadowcat/core");
      vi.mocked(listBackups).mockResolvedValue(
        listing({ restore_failed: { id: "backup-1", requested_at_ms: 5, error: "boom" } }) as never,
      );
      render(BackupManager, { context: setAppContextForTest({ serverRole: "admin" }) });
      const banner = await screen.findByRole("alert");
      expect(banner.textContent).toContain("settings.backups.restoreFailed");
    });

    it("uses the unknown-backup wording when the pending marker itself was unreadable", async () => {
      const { listBackups } = await import("@shadowcat/core");
      vi.mocked(listBackups).mockResolvedValue(
        listing({ restore_failed: { id: "", requested_at_ms: 0, error: "unreadable" } }) as never,
      );
      render(BackupManager, { context: setAppContextForTest({ serverRole: "admin" }) });
      expect((await screen.findByRole("alert")).textContent).toContain(
        "settings.backups.restoreFailedUnknown",
      );
    });
  });
});
