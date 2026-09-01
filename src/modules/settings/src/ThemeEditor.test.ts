import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, fireEvent, within } from "@testing-library/svelte";
import { setAppContextForTest } from "@shadowcat/ui-kit/test";
import {
  BUILTIN_THEMES,
  DEFAULT_THEME_ID,
  colorThemeTokenNames,
  theme,
} from "@shadowcat/ui-kit";
import ThemeEditor from "./ThemeEditor.svelte";

// The fixture's `t` is an identity echo (every key resolves to itself), so
// assertions pin keys and structure rather than resolved English. The editor
// renders token names verbatim (`--accent`); they are code identifiers, not
// translatable strings.
describe("ThemeEditor", () => {
  afterEach(() => {
    theme.previewCustom(null);
    theme.setActive(DEFAULT_THEME_ID);
    for (const id of Object.keys(theme.customThemes)) theme.deleteCustom(id);
  });

  const defaultAccent = BUILTIN_THEMES.find((t) => t.id === DEFAULT_THEME_ID)!.tokens.accent;

  function renderEditor(themeId: string | null = null, onclose: () => void = () => {}) {
    return render(ThemeEditor, {
      props: { themeId, onclose },
      context: setAppContextForTest({ role: "player" }),
    });
  }

  it("renders one color-input row per curated color token", () => {
    renderEditor();
    const inputs = screen.getAllByLabelText(/^--/);
    expect(inputs).toHaveLength(colorThemeTokenNames().length);
    for (const input of inputs) {
      expect((input as HTMLInputElement).type).toBe("color");
    }
    // Spot-check: translucency and scale tokens get no row.
    expect(screen.queryByLabelText("--scrim")).toBeNull();
    expect(screen.queryByLabelText("--space-1")).toBeNull();
  });

  it("edits preview through the controller without persisting", async () => {
    renderEditor();
    await fireEvent.input(screen.getByLabelText("--accent"), { target: { value: "#123456" } });
    expect(theme.resolved.tokens.accent).toBe("#123456");
    expect(theme.active).toBe(DEFAULT_THEME_ID);
    expect(theme.serialize().custom).toEqual({});
  });

  it("per-row reset clears the override back to the base value", async () => {
    renderEditor();
    const input = screen.getByLabelText("--accent") as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "#123456" } });
    expect(theme.resolved.tokens.accent).toBe("#123456");
    const row = input.closest(".row") as HTMLElement;
    const reset = within(row).getByRole("button", { name: "settings.theme.editor.reset" });
    await fireEvent.click(reset);
    expect(theme.resolved.tokens.accent).toBe(defaultAccent);
    expect(input.value).toBe(defaultAccent);
    expect((reset as HTMLButtonElement).disabled).toBe(true);
  });

  it("flags rows involved in a failing contrast pairing, without blocking", async () => {
    renderEditor();
    await fireEvent.input(screen.getByLabelText("settings.theme.editor.name"), {
      target: { value: "Mine" },
    });
    const surfaceBase = BUILTIN_THEMES.find((t) => t.id === DEFAULT_THEME_ID)!.tokens[
      "surface-base"
    ];
    const textInput = screen.getByLabelText("--text-primary") as HTMLInputElement;
    await fireEvent.input(textInput, { target: { value: surfaceBase } });
    const textRow = textInput.closest(".row") as HTMLElement;
    expect(within(textRow).getByText("settings.theme.editor.lowContrast")).toBeTruthy();
    // The counterpart row is flagged too, and saving stays possible.
    const surfaceInput = screen.getByLabelText("--surface-base") as HTMLInputElement;
    const surfaceRow = surfaceInput.closest(".row") as HTMLElement;
    expect(within(surfaceRow).getByText("settings.theme.editor.lowContrast")).toBeTruthy();
    const save = screen.getByRole("button", {
      name: "settings.theme.editor.save",
    }) as HTMLButtonElement;
    expect(save.disabled).toBe(false);
  });

  it("disables save until the theme has a name", async () => {
    renderEditor();
    const save = screen.getByRole("button", {
      name: "settings.theme.editor.save",
    }) as HTMLButtonElement;
    expect(save.disabled).toBe(true);
    await fireEvent.input(screen.getByLabelText("settings.theme.editor.name"), {
      target: { value: "Mine" },
    });
    expect(save.disabled).toBe(false);
  });

  it("save persists the theme, activates it, clears the preview, and closes", async () => {
    const onclose = vi.fn();
    renderEditor(null, onclose);
    await fireEvent.input(screen.getByLabelText("settings.theme.editor.name"), {
      target: { value: "Mine" },
    });
    await fireEvent.input(screen.getByLabelText("--accent"), { target: { value: "#123456" } });
    await fireEvent.click(screen.getByRole("button", { name: "settings.theme.editor.save" }));
    const ids = Object.keys(theme.customThemes);
    expect(ids).toHaveLength(1);
    expect(theme.customThemes[ids[0]!]).toEqual({
      label: "Mine",
      base: DEFAULT_THEME_ID,
      tokens: { accent: "#123456" },
    });
    expect(theme.active).toBe(`custom:${ids[0]}`);
    expect(theme.resolved.id).toBe(`custom:${ids[0]}`);
    expect(theme.resolved.tokens.accent).toBe("#123456");
    expect(onclose).toHaveBeenCalledTimes(1);
  });

  it("cancel reverts the preview and closes without persisting", async () => {
    const onclose = vi.fn();
    renderEditor(null, onclose);
    await fireEvent.input(screen.getByLabelText("--accent"), { target: { value: "#123456" } });
    expect(theme.resolved.tokens.accent).toBe("#123456");
    await fireEvent.click(screen.getByRole("button", { name: "settings.theme.editor.cancel" }));
    expect(theme.resolved.tokens.accent).toBe(defaultAccent);
    expect(theme.serialize().custom).toEqual({});
    expect(onclose).toHaveBeenCalledTimes(1);
  });

  it("editing an existing theme seeds name, base, and overrides", () => {
    theme.saveCustom("mine", {
      label: "Mine",
      base: "slate-light",
      tokens: { accent: "#123456" },
    });
    renderEditor("mine");
    expect((screen.getByLabelText("settings.theme.editor.name") as HTMLInputElement).value).toBe(
      "Mine",
    );
    expect((screen.getByLabelText("settings.theme.editor.base") as HTMLSelectElement).value).toBe(
      "slate-light",
    );
    expect((screen.getByLabelText("--accent") as HTMLInputElement).value).toBe("#123456");
    expect(theme.resolved.tokens.accent).toBe("#123456");
  });

  it("coerces a persisted non-hex color override to the base value", () => {
    theme.saveCustom("mine", { label: "Mine", base: "slate-dark", tokens: { accent: "red" } });
    renderEditor("mine");
    expect((screen.getByLabelText("--accent") as HTMLInputElement).value).toBe(defaultAccent);
  });
});

import themeEditorSource from "./ThemeEditor.svelte?raw";

describe("ThemeEditor touch and compact layout", () => {
  // jsdom evaluates neither @media (pointer: coarse) nor width breakpoints, so
  // assert the rules' presence directly in the component's source styles
  // (mirrors the touch-sizing assertion convention in GameSettingsPanel's
  // touch test).
  it("color inputs and buttons meet the 44px coarse-pointer floor", () => {
    // The coarse-pointer block must size BOTH the color inputs and the
    // buttons (reset/save/cancel) to the shared coarse target height.
    expect(themeEditorSource).toMatch(
      /@media \(pointer: coarse\)\s*\{[^]*?input\[type="color"\]\s*\{\s*min-height:\s*var\(--input-height-coarse\)/,
    );
    expect(themeEditorSource).toMatch(
      /@media \(pointer: coarse\)\s*\{[^]*?button\s*\{\s*min-height:\s*var\(--input-height-coarse\)/,
    );
  });

  it("rows stack on the compact breakpoint", () => {
    expect(themeEditorSource).toMatch(
      /@media \(max-width: 48rem\)\s*\{\s*\.row\s*\{\s*flex-direction:\s*column/,
    );
  });
});
