import { describe, it, expect, afterEach } from "vitest";
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
