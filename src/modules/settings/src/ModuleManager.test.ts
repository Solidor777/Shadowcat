import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import ModuleManager from "./ModuleManager.svelte";

vi.mock("@shadowcat/core", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@shadowcat/core")>();
  return {
    ...actual,
    listInstalledModules: vi.fn().mockResolvedValue([
      { id: "example-system", manifest: { id: "example-system" }, entry_url: "/modules/example-system/index.js" },
    ]),
    getEnabledModules: vi.fn().mockResolvedValue([]),
    setEnabledModules: vi.fn().mockResolvedValue(undefined),
  };
});

describe("ModuleManager", () => {
  it("lists installed modules and lets the GM toggle + save an enabled set", async () => {
    const { setEnabledModules } = await import("@shadowcat/core");
    render(ModuleManager, { context: setAppContextForTest({ world: "w1", role: "gm" }) });

    const checkbox = await screen.findByLabelText("example-system");
    expect((checkbox as HTMLInputElement).checked).toBe(false);

    await fireEvent.click(checkbox);
    expect((checkbox as HTMLInputElement).checked).toBe(true);

    await fireEvent.click(screen.getByText("settings.modules.save"));
    await vi.waitFor(() => expect(vi.mocked(setEnabledModules)).toHaveBeenCalledWith("w1", ["example-system"]));
  });

  it("calls ctx.reconcileInstalledModules exactly once after a successful save", async () => {
    const reconcileInstalledModules = vi.fn().mockResolvedValue(undefined);
    render(ModuleManager, {
      context: setAppContextForTest({ world: "w1", role: "gm", reconcileInstalledModules }),
    });

    const checkbox = await screen.findByLabelText("example-system");
    await fireEvent.click(checkbox);
    await fireEvent.click(screen.getByText("settings.modules.save"));

    await vi.waitFor(() => expect(reconcileInstalledModules).toHaveBeenCalledOnce());
  });

  it("does NOT call ctx.reconcileInstalledModules when setEnabledModules rejects", async () => {
    const { setEnabledModules } = await import("@shadowcat/core");
    vi.mocked(setEnabledModules).mockRejectedValueOnce(new Error("save failed"));
    const reconcileInstalledModules = vi.fn().mockResolvedValue(undefined);
    render(ModuleManager, {
      context: setAppContextForTest({ world: "w1", role: "gm", reconcileInstalledModules }),
    });

    const checkbox = await screen.findByLabelText("example-system");
    await fireEvent.click(checkbox);
    await fireEvent.click(screen.getByText("settings.modules.save"));

    await vi.waitFor(() => expect(screen.getByText("settings.modules.error")).toBeTruthy());
    expect(reconcileInstalledModules).not.toHaveBeenCalled();
  });

  it("shows an empty state when nothing is installed", async () => {
    const { listInstalledModules } = await import("@shadowcat/core");
    vi.mocked(listInstalledModules).mockResolvedValueOnce([]);
    render(ModuleManager, { context: setAppContextForTest({ world: "w1", role: "gm" }) });
    expect(await screen.findByText("settings.modules.empty")).toBeTruthy();
  });

  it("shows an error message when discovery fails", async () => {
    const { listInstalledModules } = await import("@shadowcat/core");
    vi.mocked(listInstalledModules).mockRejectedValueOnce(new Error("boom"));
    render(ModuleManager, { context: setAppContextForTest({ world: "w1", role: "gm" }) });
    expect(await screen.findByText("settings.modules.error")).toBeTruthy();
  });

  it("surfaces the error message and stops saving when setEnabledModules rejects", async () => {
    const { setEnabledModules } = await import("@shadowcat/core");
    vi.mocked(setEnabledModules).mockRejectedValueOnce(new Error("save failed"));
    render(ModuleManager, { context: setAppContextForTest({ world: "w1", role: "gm" }) });

    const checkbox = await screen.findByLabelText("example-system");
    await fireEvent.click(checkbox);
    const saveButton = screen.getByText("settings.modules.save") as HTMLButtonElement;
    await fireEvent.click(saveButton);

    expect(await screen.findByText("settings.modules.error")).toBeTruthy();
    // "saving" must reset to false — the UI must not keep claiming a save is
    // still in flight (and lying about persisted state) after it failed.
    await vi.waitFor(() => expect(saveButton.disabled).toBe(false));
  });

  it("keys the toggle/save identity on the canonical folder id (info.id), not manifest.id, when they differ", async () => {
    const { listInstalledModules, setEnabledModules } = await import("@shadowcat/core");
    vi.mocked(listInstalledModules).mockResolvedValueOnce([
      { id: "folder-name", manifest: { id: "declared-manifest-id" }, entry_url: "/modules/folder-name/index.js" },
    ]);
    render(ModuleManager, { context: setAppContextForTest({ world: "w1", role: "gm" }) });

    // Display label is the manifest's declared id (module NAME) — that's the
    // human-facing part. The toggle/save identity below is the folder id.
    const checkbox = await screen.findByLabelText("declared-manifest-id");
    await fireEvent.click(checkbox);
    await fireEvent.click(screen.getByText("settings.modules.save"));
    await vi.waitFor(() =>
      expect(vi.mocked(setEnabledModules)).toHaveBeenCalledWith("w1", ["folder-name"]),
    );
  });
});
