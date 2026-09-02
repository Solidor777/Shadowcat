import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import { BUILTIN_THEMES, DEFAULT_THEME_ID, theme } from "@shadowcat/ui-kit";
import Settings from "./Settings.svelte";

// The fixture's `t` is an identity echo (every key resolves to itself), so
// assertions pin keys and structure rather than resolved English.
describe("Settings theme picker", () => {
  afterEach(() => {
    theme.setActive(DEFAULT_THEME_ID);
    for (const id of Object.keys(theme.customThemes)) theme.deleteCustom(id);
  });

  it("renders a theme select listing every built-in theme by its label key", () => {
    render(Settings, { context: setAppContextForTest({ role: "player" }) });

    const select = screen.getByLabelText("settings.theme.label") as HTMLSelectElement;
    const options = Array.from(select.options).map((o) => ({ value: o.value, text: o.textContent }));
    for (const builtin of BUILTIN_THEMES) {
      expect(options).toContainEqual({ value: builtin.id, text: builtin.labelKey });
    }
    expect(select.value).toBe(DEFAULT_THEME_ID);
  });

  it("lists saved custom themes with their verbatim labels", () => {
    theme.saveCustom("mine", { label: "My Theme", base: "slate-dark", tokens: {} });
    render(Settings, { context: setAppContextForTest({ role: "player" }) });

    const select = screen.getByLabelText("settings.theme.label") as HTMLSelectElement;
    const values = Array.from(select.options).map((o) => o.value);
    expect(values).toContain("custom:mine");
    const option = Array.from(select.options).find((o) => o.value === "custom:mine")!;
    expect(option.textContent).toBe("My Theme");
  });

  it("changing the selection calls through to the theme controller", async () => {
    render(Settings, { context: setAppContextForTest({ role: "player" }) });

    const select = screen.getByLabelText("settings.theme.label") as HTMLSelectElement;
    await fireEvent.change(select, { target: { value: "slate-light" } });
    expect(theme.active).toBe("slate-light");
  });
});

describe("Settings custom theme management", () => {
  afterEach(() => {
    theme.previewCustom(null);
    theme.setActive(DEFAULT_THEME_ID);
    for (const id of Object.keys(theme.customThemes)) theme.deleteCustom(id);
  });

  it("opens the theme editor from the new-theme button", async () => {
    render(Settings, { context: setAppContextForTest({ role: "player" }) });
    expect(screen.queryByText("settings.theme.editor.heading")).toBeNull();
    await fireEvent.click(screen.getByRole("button", { name: "settings.theme.editor.new" }));
    expect(screen.getByText("settings.theme.editor.heading")).toBeTruthy();
  });

  it("lists each custom theme with edit and delete buttons", () => {
    theme.saveCustom("mine", { label: "My Theme", base: "slate-dark", tokens: {} });
    render(Settings, { context: setAppContextForTest({ role: "player" }) });
    expect(screen.getByRole("button", { name: "settings.theme.editor.edit" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "settings.theme.editor.delete" })).toBeTruthy();
  });

  it("the edit button opens the editor seeded with that theme", async () => {
    theme.saveCustom("mine", { label: "My Theme", base: "slate-dark", tokens: {} });
    render(Settings, { context: setAppContextForTest({ role: "player" }) });
    await fireEvent.click(screen.getByRole("button", { name: "settings.theme.editor.edit" }));
    expect(
      (screen.getByLabelText("settings.theme.editor.name") as HTMLInputElement).value,
    ).toBe("My Theme");
  });

  it("delete asks for confirmation, then removes the theme and falls back when active", async () => {
    theme.saveCustom("mine", { label: "My Theme", base: "slate-dark", tokens: {} });
    theme.setActive("custom:mine");
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);
    render(Settings, { context: setAppContextForTest({ role: "player" }) });
    await fireEvent.click(screen.getByRole("button", { name: "settings.theme.editor.delete" }));
    expect(confirmSpy).toHaveBeenCalledWith("settings.theme.editor.deleteConfirm");
    expect(theme.customThemes).toEqual({});
    expect(theme.active).toBe(DEFAULT_THEME_ID);
    expect(screen.queryByRole("button", { name: "settings.theme.editor.delete" })).toBeNull();
  });

  it("a declined confirm leaves the theme in place", async () => {
    theme.saveCustom("mine", { label: "My Theme", base: "slate-dark", tokens: {} });
    vi.spyOn(window, "confirm").mockReturnValue(false);
    render(Settings, { context: setAppContextForTest({ role: "player" }) });
    await fireEvent.click(screen.getByRole("button", { name: "settings.theme.editor.delete" }));
    expect(Object.keys(theme.customThemes)).toEqual(["mine"]);
  });
});
