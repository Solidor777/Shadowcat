<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
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

  let tagDraft = $state("");

  /** Commits the tag input as a chip (deduplicated, non-empty).
   * @example
   * ```
   * // private function; wired to the tag input's Enter handler below
   * commitTag();
   * ```
   */
  function commitTag(): void {
    const tag = tagDraft.trim();
    tagDraft = "";
    if (!tag || filter.tags.includes(tag)) return;
    onChange({ ...filter, tags: [...filter.tags, tag] });
  }
</script>

<div class="filter-bar">
  <input
    data-testid="filter-name"
    type="search"
    placeholder={filter.nameIsRegex ? t("assetBrowser.filterRegex") : t("assetBrowser.filterName")}
    value={filter.name}
    oninput={(e) => onChange({ ...filter, name: e.currentTarget.value })}
  />
  <button
    type="button"
    data-testid="filter-regex-toggle"
    class="regex"
    class:active={filter.nameIsRegex}
    aria-pressed={filter.nameIsRegex}
    title={t("assetBrowser.filterRegex")}
    onclick={() => onChange({ ...filter, nameIsRegex: !filter.nameIsRegex })}
  >.*</button>

  <span class="tags">
    {#each filter.tags as tag (tag)}
      <span class="chip">
        {tag}
        <button
          type="button"
          data-testid={"filter-tag-remove-" + tag}
          aria-label={t("assetBrowser.removeTag", { tag })}
          onclick={() => onChange({ ...filter, tags: filter.tags.filter((x) => x !== tag) })}
        >×</button>
      </span>
    {/each}
    <input
      data-testid="filter-tag-input"
      type="text"
      placeholder={t("assetBrowser.filterTags")}
      bind:value={tagDraft}
      onkeydown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          commitTag();
        }
      }}
    />
  </span>

  <select
    data-testid="filter-kind"
    value={filter.kind ?? ""}
    onchange={(e) =>
      onChange({
        ...filter,
        kind: e.currentTarget.value === "" ? undefined : (e.currentTarget.value as "image" | "other"),
      })}
  >
    <option value="">{t("assetBrowser.kindAll")}</option>
    <option value="image">{t("assetBrowser.kindImage")}</option>
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
      background: var(--accent, #46f);
      color: var(--text-on-accent, #fff);
    }
  }
  .tags {
    display: inline-flex;
    flex-wrap: wrap;
    gap: 0.25rem;
    align-items: center;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 0.125rem;
    padding: 0.125rem 0.375rem;
    border: 1px solid var(--border);
    border-radius: 1rem;
    font-size: 0.8rem;
    button {
      min-height: 0;
      border: none;
      background: none;
      cursor: pointer;
      color: var(--text-muted);
    }
  }
</style>
