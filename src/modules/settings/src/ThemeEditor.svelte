<script lang="ts">
  import { onDestroy, untrack } from "svelte";
  import {
    BUILTIN_THEMES,
    DEFAULT_THEME_ID,
    colorThemeTokenNames,
    contrastWarnings,
    getAppContext,
    theme,
    type ThemeTokenName,
  } from "@shadowcat/ui-kit";

  /** ThemeEditor props. */
  interface Props {
    /** Id of the custom theme being edited, or `null` for a new theme. Read
     * once at mount — the host remounts the editor when the edit target
     * changes. */
    themeId: string | null;
    /** Called when the editor closes, by save or by cancel. */
    onclose: () => void;
  }
  const { themeId, onclose }: Props = $props();
  const { t } = getAppContext();

  /** The editable color tokens, derived from the built-in theme data. */
  const colorTokens = colorThemeTokenNames();

  /** The only color syntax `<input type="color">` accepts; the theme data pins
   * every curated token to it in every built-in. */
  const HEX_COLOR = /^#[0-9a-f]{6}$/i;

  const existing = themeId !== null ? theme.customThemes[themeId] : undefined;

  let name = $state(existing?.label ?? "");
  let base = $state(
    existing && BUILTIN_THEMES.some((b) => b.id === existing.base)
      ? existing.base
      : DEFAULT_THEME_ID,
  );
  let overrides = $state<Partial<Record<ThemeTokenName, string>>>(
    sanitizeLoadedOverrides(existing?.tokens ?? {}),
  );

  /**
   * Prepares a persisted override map for editing: a curated color token whose
   * value a color input cannot represent (a hand-edited non-`#rrggbb` string)
   * falls back to the base theme's value rather than poisoning the input.
   * Overrides for non-color tokens pass through untouched — the editor does
   * not own them and must not silently discard them on save.
   * @param tokens The persisted override map.
   * @returns The override map with uneditable color values dropped.
   * @example
   * ```
   * // private function; not part of the public API — runs once at mount
   * sanitizeLoadedOverrides({ accent: "red" }); // {}
   * ```
   */
  function sanitizeLoadedOverrides(
    tokens: Partial<Record<ThemeTokenName, string>>,
  ): Partial<Record<ThemeTokenName, string>> {
    const clean: Partial<Record<ThemeTokenName, string>> = {};
    const editable = new Set<string>(colorTokens);
    for (const [token, value] of Object.entries(tokens)) {
      if (!editable.has(token) || HEX_COLOR.test(value)) {
        clean[token as ThemeTokenName] = value;
      }
    }
    return clean;
  }

  const baseTheme = $derived(
    BUILTIN_THEMES.find((b) => b.id === base) ?? BUILTIN_THEMES[0]!,
  );
  const effective = $derived<Record<ThemeTokenName, string>>({
    ...baseTheme.tokens,
    ...overrides,
  });
  const warnings = $derived(contrastWarnings(effective));
  /** Every token participating in a failing pairing, mapped to the tokens it
   * fails against. A failing pairing flags BOTH rows: either endpoint is a
   * valid place to fix it. */
  const warningsByToken = $derived.by(() => {
    const map = new Map<ThemeTokenName, Set<ThemeTokenName>>();
    for (const w of warnings) {
      if (!map.has(w.fg)) map.set(w.fg, new Set());
      map.get(w.fg)!.add(w.bg);
      if (!map.has(w.bg)) map.set(w.bg, new Set());
      map.get(w.bg)!.add(w.fg);
    }
    return map;
  });

  // Live preview: every draft change rides the controller's transient preview
  // seam, so the whole app (and every open window) shows the draft without
  // persisting anything. The controller call is untracked: `previewCustom`
  // reads controller `$state` internally (via `resolved`), and tracking those
  // reads would loop the effect against its own `#preview` write.
  const previewOwner = {};
  $effect(() => {
    const draft = { label: name, base, tokens: overrides };
    untrack(() => theme.previewCustom(draft, previewOwner));
  });
  // Teardown must not mutate the controller synchronously: a controller
  // `$state` read during same-flush component teardown observes the pre-flush
  // value, so a `#changed()` fired from `onDestroy` lets subscribers serialize
  // and persist state the current flush already superseded. Deferring to a
  // microtask runs the clear after the flush, with fresh state; scoping it to
  // `previewOwner` keeps a successor editor's preview (mounted by the same
  // flush) intact.
  onDestroy(() => {
    queueMicrotask(() => theme.clearPreview(previewOwner));
  });

  /**
   * Sets one token override from its color input.
   * @param token The token being overridden.
   * @param value The input's `#rrggbb` value.
   * @example
   * ```
   * // private function; not part of the public API — wired to each row's color input
   * declare const token: ThemeTokenName;
   * setOverride(token, "#123456");
   * ```
   */
  function setOverride(token: ThemeTokenName, value: string): void {
    overrides = { ...overrides, [token]: value };
  }

  /**
   * Clears one token's override, returning the row to the base theme's value.
   * @param token The token to reset.
   * @example
   * ```
   * // private function; not part of the public API — wired to each row's reset button
   * declare const token: ThemeTokenName;
   * resetOverride(token);
   * ```
   */
  function resetOverride(token: ThemeTokenName): void {
    const next = { ...overrides };
    delete next[token];
    overrides = next;
  }

  /**
   * The row-level contrast warning text: which tokens this row's token fails
   * against. Advisory only — saving is never blocked.
   * @param token The row's token.
   * @returns The localized warning text.
   * @example
   * ```
   * // private function; not part of the public API — called per flagged row
   * declare const token: ThemeTokenName;
   * warningText(token);
   * ```
   */
  function warningText(token: ThemeTokenName): string {
    const others = [...(warningsByToken.get(token) ?? [])]
      .map((n) => `--${n}`)
      .join(", ");
    return t("settings.theme.editor.lowContrast", { tokens: others });
  }

  /**
   * Persists the draft as a custom theme and activates it. Activating is
   * deliberate: the preview has been showing this draft all along, so the
   * active theme switching to the saved theme keeps what the user sees stable
   * across the save. Editing an existing theme saves under its own id; a new
   * theme gets a fresh uuid. Closes the editor afterwards.
   * @example
   * ```
   * // private function; not part of the public API — wired to the save button
   * save();
   * ```
   */
  function save(): void {
    const id = themeId ?? crypto.randomUUID();
    theme.saveCustom(id, { label: name.trim(), base, tokens: overrides });
    theme.setActive(`custom:${id}`);
    theme.clearPreview(previewOwner);
    onclose();
  }

  /**
   * Discards the draft: clears the preview (documents revert to the genuinely
   * active theme) and closes without persisting anything.
   * @example
   * ```
   * // private function; not part of the public API — wired to the cancel button
   * cancel();
   * ```
   */
  function cancel(): void {
    theme.clearPreview(previewOwner);
    onclose();
  }
</script>

<section class="theme-editor">
  <h3>{t("settings.theme.editor.heading")}</h3>
  <label class="field">
    {t("settings.theme.editor.name")}
    <input type="text" bind:value={name} />
  </label>
  <label class="field">
    {t("settings.theme.editor.base")}
    <select bind:value={base}>
      {#each BUILTIN_THEMES as builtin (builtin.id)}
        <option value={builtin.id}>{t(builtin.labelKey)}</option>
      {/each}
    </select>
  </label>
  <div class="rows">
    {#each colorTokens as token (token)}
      <div class="row">
        <label class="token">
          <span class="token-name">--{token}</span>
          <input
            type="color"
            value={effective[token]}
            oninput={(e) => setOverride(token, e.currentTarget.value)}
          />
        </label>
        <button
          type="button"
          class="reset"
          disabled={!(token in overrides)}
          onclick={() => resetOverride(token)}
        >
          {t("settings.theme.editor.reset")}
        </button>
        {#if warningsByToken.has(token)}
          <p class="contrast-warning">{warningText(token)}</p>
        {/if}
      </div>
    {/each}
  </div>
  <div class="actions">
    <button type="button" onclick={save} disabled={name.trim() === ""}>
      {t("settings.theme.editor.save")}
    </button>
    <button type="button" onclick={cancel}>{t("settings.theme.editor.cancel")}</button>
  </div>
</section>

<style lang="scss">
  .theme-editor {
    display: grid;
    gap: var(--space-3);
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-2);
  }
  .theme-editor h3 {
    margin: 0;
  }
  .field {
    display: grid;
    gap: var(--space-1);
  }
  .rows {
    display: grid;
    gap: var(--space-2);
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .token {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex: 1;
  }
  .token-name {
    flex: 1;
    font-family: monospace;
    font-size: var(--font-size-sm);
    color: var(--text-muted);
  }
  .contrast-warning {
    flex-basis: 100%;
    margin: 0;
    color: var(--warning);
    font-size: var(--font-size-sm);
  }
  .actions {
    display: flex;
    gap: var(--space-2);
  }
  input[type="color"] {
    width: 48px;
    height: 32px;
    padding: 0;
    border: 1px solid var(--border);
    background: none;
  }
  input[type="text"],
  select,
  button {
    min-height: 32px;
  }
  @media (pointer: coarse) {
    input[type="color"] {
      min-height: var(--input-height-coarse);
      min-width: var(--input-height-coarse);
    }
    button {
      min-height: var(--input-height-coarse);
    }
  }
  @media (max-width: 48rem) {
    .row {
      flex-direction: column;
      align-items: stretch;
    }
    .contrast-warning {
      flex-basis: auto;
    }
  }
</style>
