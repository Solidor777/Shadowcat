<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import type { SceneLevel } from "@shadowcat/core";

  const ctx = getAppContext();
  const t = ctx.t;

  let {
    levels,
    onCommit,
  }: {
    /** The scene's currently-authored floors. */
    levels: SceneLevel[];
    /** Called with the WHOLE updated array on any add/remove/edit — every mutation clones the
     * array, mutates the clone, and calls this once. */
    onCommit: (next: SceneLevel[]) => void;
  } = $props();

  /**
   * Clones `levels`, applies `mutate` to the clone, and commits the result.
   * @param mutate Mutates the cloned array in place (push/splice/field edit).
   * @returns Nothing; calls `onCommit` as a side effect.
   * @example
   * ```
   * declare const next: SceneLevel[];
   * // private helper; not part of the public API — invoked from every row control
   * commit((clone) => clone.push(next[0]));
   * ```
   */
  function commit(mutate: (clone: SceneLevel[]) => void): void {
    const clone = structuredClone(levels);
    mutate(clone);
    onCommit(clone);
  }

  /**
   * Appends a new level row with a generated id and a default 10-unit band stacked directly
   * above every existing level's `top` (`[0, 10)` when there are none yet). A fixed `[0, 10)`
   * default for every row would collide with an already-authored level's identical default band,
   * and the server's `SceneEngine::validate` rejects overlapping bands outright — stacking keeps
   * every freshly-added row valid without requiring the author to immediately re-edit it.
   * @returns Nothing; dispatches the updated array as a side effect.
   * @example
   * ```
   * // private helper; not part of the public API — invoked from the "add" button
   * addLevel();
   * ```
   */
  function addLevel(): void {
    commit((clone) => {
      const bottom = clone.reduce((max, l) => Math.max(max, l.top), 0);
      clone.push({ id: crypto.randomUUID(), name: "", bottom, top: bottom + 10, background: null });
    });
  }

  /**
   * Removes one level row by id.
   * @param id The level id to remove.
   * @returns Nothing; dispatches the updated array as a side effect.
   * @example
   * ```
   * declare const id: string;
   * // private helper; not part of the public API — invoked from a row's "remove" button
   * removeLevel(id);
   * ```
   */
  function removeLevel(id: string): void {
    commit((clone) => {
      const i = clone.findIndex((l) => l.id === id);
      if (i !== -1) clone.splice(i, 1);
    });
  }

  /**
   * Edits one field of one level row by id.
   * @param id The level id to edit.
   * @param field The field to overwrite.
   * @param value The new value.
   * @returns Nothing; dispatches the updated array as a side effect.
   * @example
   * ```
   * declare const id: string;
   * // private helper; not part of the public API — invoked from a row's inputs
   * editLevel(id, "name", "Upper Floor");
   * ```
   */
  function editLevel<K extends keyof SceneLevel>(id: string, field: K, value: SceneLevel[K]): void {
    commit((clone) => {
      const row = clone.find((l) => l.id === id);
      if (row) row[field] = value;
    });
  }

  /**
   * Opens the asset picker for a level's background, writing the picked (or cleared) asset id.
   * @param id The level id being edited.
   * @returns Nothing; dispatches the updated array as a side effect once the picker resolves.
   * @example
   * ```
   * declare const id: string;
   * // private helper; not part of the public API — invoked from a row's background button
   * pickBackground(id);
   * ```
   */
  function pickBackground(id: string): void {
    void ctx.pickAsset({ kind: "image" }).then((assetId) => {
      if (assetId === null) return;
      editLevel(id, "background", assetId);
    });
  }
</script>

<section class="levels-editor" data-testid="levels-editor" aria-label={t("levels.editorTitle")}>
  {#each levels as level (level.id)}
    <div class="level-row" data-testid="level-row" data-level-id={level.id}>
      <input
        type="text"
        data-testid="level-name"
        aria-label={t("levels.name")}
        value={level.name}
        oninput={(e) => editLevel(level.id, "name", e.currentTarget.value)}
      />
      <input
        type="number"
        data-testid="level-bottom"
        aria-label={t("levels.bottom")}
        aria-invalid={level.bottom >= level.top}
        value={level.bottom}
        oninput={(e) => editLevel(level.id, "bottom", Number(e.currentTarget.value))}
      />
      <input
        type="number"
        data-testid="level-top"
        aria-label={t("levels.top")}
        aria-invalid={level.bottom >= level.top}
        value={level.top}
        oninput={(e) => editLevel(level.id, "top", Number(e.currentTarget.value))}
      />
      <button type="button" data-testid="level-background" onclick={() => pickBackground(level.id)}>
        {t("levels.background")}
      </button>
      {#if level.background}
        <button
          type="button"
          data-testid="level-background-clear"
          onclick={() => editLevel(level.id, "background", null)}
        >
          {t("levels.backgroundClear")}
        </button>
      {/if}
      <button type="button" data-testid="level-remove" onclick={() => removeLevel(level.id)}>
        {t("levels.removeLevel")}
      </button>
      {#if level.bottom >= level.top}
        <span class="level-invalid" data-testid="level-invalid" role="alert">
          {t("levels.invalidBand")}
        </span>
      {/if}
    </div>
  {/each}
  <button type="button" data-testid="level-add" onclick={addLevel}>{t("levels.addLevel")}</button>
</section>

<style lang="scss">
  .levels-editor {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .level-row {
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }
  .level-row input[type="text"] {
    flex: 1 1 auto;
  }
  .level-row input[type="number"] {
    width: 5rem;
  }
  .level-row input,
  .level-row button,
  .levels-editor > button {
    min-height: var(--input-height-coarse);
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    background: var(--surface-raised);
    color: var(--text-primary);
  }
  .level-row button,
  .levels-editor > button {
    cursor: pointer;
  }
  .level-invalid {
    color: var(--danger);
  }
</style>
