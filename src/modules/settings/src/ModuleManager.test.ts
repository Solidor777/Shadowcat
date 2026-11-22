import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import ModuleManager from "./ModuleManager.svelte";

vi.mock("@shadowcat/core", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@shadowcat/core")>();
  return {
    ...actual,
    listInstalledModules: vi.fn().mockResolvedValue([
      { id: "example-system", manifest: { id: "example-system" }, entry_url: "/modules/example-system/index.js", has_validators: false, validator_load_error: null, engine_compat_ok: true },
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
    await vi.waitFor(() => expect(vi.mocked(setEnabledModules)).toHaveBeenCalledWith("w1", [{ id: "example-system", validators_enabled: false }]));
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
      { id: "folder-name", manifest: { id: "declared-manifest-id" }, entry_url: "/modules/folder-name/index.js", has_validators: false, validator_load_error: null, engine_compat_ok: true },
    ]);
    render(ModuleManager, { context: setAppContextForTest({ world: "w1", role: "gm" }) });

    // Display label is the manifest's declared id (module NAME) — that's the
    // human-facing part. The toggle/save identity below is the folder id.
    const checkbox = await screen.findByLabelText("declared-manifest-id");
    await fireEvent.click(checkbox);
    await fireEvent.click(screen.getByText("settings.modules.save"));
    await vi.waitFor(() =>
      expect(vi.mocked(setEnabledModules)).toHaveBeenCalledWith("w1", [
        { id: "folder-name", validators_enabled: false },
      ]),
    );
  });

  it("shows the validators toggle only for an enabled module that declares validators", async () => {
    const { listInstalledModules, getEnabledModules } = await import("@shadowcat/core");
    vi.mocked(listInstalledModules).mockResolvedValueOnce([
      { id: "example-system", manifest: { id: "example-system" }, entry_url: "/modules/example-system/index.js", has_validators: true, validator_load_error: null, engine_compat_ok: true },
    ]);
    vi.mocked(getEnabledModules).mockResolvedValueOnce([{ id: "example-system", validators_enabled: false }]);
    render(ModuleManager, { context: setAppContextForTest({ world: "w1", role: "gm" }) });

    expect(await screen.findByLabelText("settings.modules.runValidators")).toBeTruthy();
  });

  it("saves validators_enabled when the sandbox toggle is flipped", async () => {
    const { listInstalledModules, getEnabledModules, setEnabledModules } = await import("@shadowcat/core");
    vi.mocked(listInstalledModules).mockResolvedValueOnce([
      { id: "example-system", manifest: { id: "example-system" }, entry_url: "/modules/example-system/index.js", has_validators: true, validator_load_error: null, engine_compat_ok: true },
    ]);
    vi.mocked(getEnabledModules).mockResolvedValueOnce([{ id: "example-system", validators_enabled: false }]);
    render(ModuleManager, { context: setAppContextForTest({ world: "w1", role: "gm" }) });

    const validatorsToggle = await screen.findByLabelText("settings.modules.runValidators");
    await fireEvent.click(validatorsToggle);
    await fireEvent.click(screen.getByText("settings.modules.save"));

    await vi.waitFor(() =>
      expect(vi.mocked(setEnabledModules)).toHaveBeenCalledWith("w1", [
        { id: "example-system", validators_enabled: true },
      ]),
    );
  });

  it("renders a status line for a module with a stored failure, and none for a module without one", async () => {
    const { listInstalledModules } = await import("@shadowcat/core");
    vi.mocked(listInstalledModules).mockResolvedValueOnce([
      { id: "broken", manifest: { id: "broken" }, entry_url: "/modules/broken/index.js", has_validators: false, validator_load_error: null, engine_compat_ok: true },
      { id: "fine", manifest: { id: "fine" }, entry_url: "/modules/fine/index.js", has_validators: false, validator_load_error: null, engine_compat_ok: true },
    ]);
    render(ModuleManager, {
      context: setAppContextForTest({
        world: "w1",
        role: "gm",
        moduleStatus: new Map([["broken", { id: "broken", status: "failed", error: "boom" }]]),
      }),
    });

    await screen.findByLabelText("broken");
    // Exactly one status line: the healthy module renders no empty placeholder beside it.
    expect(screen.getAllByTestId("module-status-error")).toHaveLength(1);
    expect(screen.getByTestId("module-status-error").textContent).toContain("settings.modules.activationFailed");
  });

  it("uses the load-failure message for a load-failed status", async () => {
    render(ModuleManager, {
      context: setAppContextForTest({
        world: "w1",
        role: "gm",
        moduleStatus: new Map([["example-system", { id: "example-system", status: "load-failed", error: "bad import" }]]),
      }),
    });
    await screen.findByLabelText("example-system");
    expect(screen.getByTestId("module-status-error").textContent).toContain("settings.modules.loadFailed");
  });

  it("disables the checkbox of a not-enabled module the server judged incompatible and says why", async () => {
    const { listInstalledModules } = await import("@shadowcat/core");
    vi.mocked(listInstalledModules).mockResolvedValueOnce([
      { id: "too-new", manifest: { id: "too-new", engines: { shadowcat: "^99.0.0" } }, entry_url: "/modules/too-new/index.js", has_validators: false, validator_load_error: null, engine_compat_ok: false },
    ]);
    render(ModuleManager, { context: setAppContextForTest({ world: "w1", role: "gm" }) });

    const checkbox = (await screen.findByLabelText("too-new")) as HTMLInputElement;
    expect(checkbox.disabled).toBe(true);
    expect(screen.getByTestId("module-incompatible").textContent).toContain("settings.modules.incompatible");
  });

  it("keeps an already-enabled module that became incompatible checked, and lets the GM uncheck it", async () => {
    const { listInstalledModules, getEnabledModules } = await import("@shadowcat/core");
    vi.mocked(listInstalledModules).mockResolvedValueOnce([
      { id: "too-new", manifest: { id: "too-new", engines: { shadowcat: "^99.0.0" } }, entry_url: "/modules/too-new/index.js", has_validators: false, validator_load_error: null, engine_compat_ok: false },
    ]);
    vi.mocked(getEnabledModules).mockResolvedValueOnce([{ id: "too-new", validators_enabled: false }]);
    render(ModuleManager, { context: setAppContextForTest({ world: "w1", role: "gm" }) });

    const checkbox = (await screen.findByLabelText("too-new")) as HTMLInputElement;
    expect(checkbox.checked).toBe(true);
    // The server re-checks compatibility on every save and rejects the whole batch while an
    // incompatible id is in it, so the GM must be able to remove it.
    expect(checkbox.disabled).toBe(false);
  });

  it("shows no incompatibility line for a compatible module", async () => {
    render(ModuleManager, { context: setAppContextForTest({ world: "w1", role: "gm" }) });
    await screen.findByLabelText("example-system");
    expect(screen.queryByTestId("module-incompatible")).toBeNull();
  });

  describe("pre-save satisfiability", () => {
    /** Two installed modules where `mod-a` declares a conflict with `mod-b`. */
    async function installConflictingPair(): Promise<void> {
      const { listInstalledModules } = await import("@shadowcat/core");
      vi.mocked(listInstalledModules).mockResolvedValueOnce([
        { id: "mod-a", manifest: { id: "mod-a", version: "1.0.0", dependencies: {}, conflicts: ["mod-b"] }, entry_url: "/modules/mod-a/index.js", has_validators: false, validator_load_error: null, engine_compat_ok: true },
        { id: "mod-b", manifest: { id: "mod-b", version: "1.0.0", dependencies: {} }, entry_url: "/modules/mod-b/index.js", has_validators: false, validator_load_error: null, engine_compat_ok: true },
      ]);
    }
    /** A translator that exposes its parameters so a test can read the ids a message names. */
    const tParams = (k: string, p?: Record<string, unknown>): string => `${k} ${JSON.stringify(p ?? {})}`;

    it("disables Save and names both modules while a conflicting pair is checked, and re-enables it on uncheck", async () => {
      await installConflictingPair();
      render(ModuleManager, { context: setAppContextForTest({ world: "w1", role: "gm", t: tParams }) });
      const a = await screen.findByLabelText("mod-a");
      const b = await screen.findByLabelText("mod-b");
      const save = (): HTMLButtonElement => screen.getByText(/settings\.modules\.save/) as HTMLButtonElement;

      await fireEvent.click(a);
      expect(save().disabled).toBe(false);
      expect(screen.queryAllByTestId("module-satisfiability-issue")).toHaveLength(0);

      await fireEvent.click(b);
      expect(save().disabled).toBe(true);
      const line = screen.getByTestId("module-satisfiability-issue").textContent ?? "";
      expect(line).toContain("settings.modules.conflict");
      expect(line).toContain("mod-a");
      expect(line).toContain("mod-b");

      await fireEvent.click(b);
      expect(save().disabled).toBe(false);
      expect(screen.queryAllByTestId("module-satisfiability-issue")).toHaveLength(0);
    });

    it("reports a dependency on a module that is not checked, and accepts one a running module satisfies", async () => {
      const { listInstalledModules } = await import("@shadowcat/core");
      const entry = {
        id: "needy",
        manifest: { id: "needy", version: "1.0.0", dependencies: { "core-ui": "^1.0.0" } },
        entry_url: "/modules/needy/index.js",
        has_validators: false,
        validator_load_error: null,
        engine_compat_ok: true,
      };
      vi.mocked(listInstalledModules).mockResolvedValueOnce([entry]);
      render(ModuleManager, { context: setAppContextForTest({ world: "w1", role: "gm", t: tParams }) });
      await fireEvent.click(await screen.findByLabelText("needy"));
      expect(screen.getByTestId("module-satisfiability-issue").textContent).toContain("settings.modules.dependencyUnmet");
    });

    it("accepts a dependency a running first-party module satisfies", async () => {
      const { listInstalledModules } = await import("@shadowcat/core");
      vi.mocked(listInstalledModules).mockResolvedValueOnce([
        { id: "needy", manifest: { id: "needy", version: "1.0.0", dependencies: { "core-ui": "^1.0.0" } }, entry_url: "/modules/needy/index.js", has_validators: false, validator_load_error: null, engine_compat_ok: true },
      ]);
      render(ModuleManager, {
        context: setAppContextForTest({
          world: "w1",
          role: "gm",
          activeModuleManifests: () => [{ id: "core-ui", version: "1.2.0", dependencies: {} }],
        }),
      });
      await fireEvent.click(await screen.findByLabelText("needy"));
      expect(screen.queryAllByTestId("module-satisfiability-issue")).toHaveLength(0);
      expect((screen.getByText("settings.modules.save") as HTMLButtonElement).disabled).toBe(false);
    });
  });
});
