<script lang="ts">
  import { getAppContext, TagChips } from "@shadowcat/ui-kit";
  import type { FilterState } from "./filterState";

  let {
    filter,
    onChange,
  }: {
    /** The current filter (owned by the browser). */
    filter: FilterState;
    /** Called with the full next filter on any control change. */
    onChange: (next: FilterState) => void;
  } = $props();

  const { t } = getAppContext();

  /** Toggles a quick-filter tag chip on/off (the same state a hand-typed chip drives).
   * @param tag The tag to add when absent or remove when present.
   * @example
   * ```
   * // private function; wired to the VFX quick-filter chip below
   * toggleTag("vfx");
   * ```
   */
  function toggleTag(tag: string): void {
    onChange({
      ...filter,
      tags: filter.tags.includes(tag)
        ? filter.tags.filter((x) => x !== tag)
        : [...filter.tags, tag],
    });
  }
</script>

<div class="filter-bar">
  <input
    data-testid="filter-query"
    type="search"
    placeholder={filter.queryIsRegex ? t("assetBrowser.filterRegex") : t("assetBrowser.filterQuery")}
    value={filter.query}
    oninput={(e) => onChange({ ...filter, query: e.currentTarget.value })}
  />
  <button
    type="button"
    data-testid="filter-regex-toggle"
    class="regex"
    class:active={filter.queryIsRegex}
    aria-pressed={filter.queryIsRegex}
    title={t("assetBrowser.filterRegex")}
    onclick={() => onChange({ ...filter, queryIsRegex: !filter.queryIsRegex })}
  >.*</button>

  <span class="tags">
    <button
      type="button"
      class="chip quick"
      data-testid="filter-vfx"
      class:active={filter.tags.includes("vfx")}
      aria-pressed={filter.tags.includes("vfx")}
      onclick={() => toggleTag("vfx")}
    >{t("assetBrowser.vfxFilter")}</button>
    <TagChips
      tags={filter.tags}
      onChange={(tags) => onChange({ ...filter, tags })}
      placeholder={t("assetBrowser.filterTags")}
      removeLabel={(tag) => t("assetBrowser.removeTag", { tag })}
      testIdPrefix="filter-tag"
    />
  </span>

  <select
    data-testid="filter-kind"
    value={filter.kind ?? ""}
    onchange={(e) =>
      onChange({
        ...filter,
        kind: e.currentTarget.value === "" ? undefined : (e.currentTarget.value as "image" | "other" | "audio"),
      })}
  >
    <option value="">{t("assetBrowser.kindAll")}</option>
    <option value="image">{t("assetBrowser.kindImage")}</option>
    <option value="audio">{t("assetBrowser.kindAudio")}</option>
    <option value="other">{t("assetBrowser.kindOther")}</option>
  </select>

  <select
    data-testid="filter-sort"
    value={filter.sort}
    onchange={(e) => onChange({ ...filter, sort: e.currentTarget.value as FilterState["sort"] })}
  >
    <option value="created">{t("assetBrowser.sortCreated")}</option>
    <option value="name">{t("assetBrowser.sortName")}</option>
    <option value="size">{t("assetBrowser.sortSize")}</option>
  </select>
</div>

<style lang="scss">
  .filter-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.375rem;
    padding: 0.375rem;
    border-bottom: 1px solid var(--border);

    input,
    select,
    button {
      min-height: 2rem;
    }
  }
  .regex {
    font-family: monospace;
    padding: 0 0.5rem;
    &.active {
      background: var(--accent);
      color: var(--on-accent);
    }
  }
  .tags {
    display: inline-flex;
    flex-wrap: wrap;
    gap: 0.25rem;
    align-items: center;
  }
  button.chip.quick {
    display: inline-flex;
    align-items: center;
    padding: 0.125rem 0.375rem;
    border: 1px solid var(--border);
    border-radius: 1rem;
    font-size: 0.8rem;
    cursor: pointer;
    background: none;
    color: var(--text);
    &.active {
      background: var(--accent);
      color: var(--on-accent);
      border-color: var(--accent);
    }
  }
</style>
