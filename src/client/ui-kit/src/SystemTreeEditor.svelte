<script lang="ts">
  import { getAppContext } from "./appContext";
  import { getPointer, type WireDocument } from "@shadowcat/core";
  import { setField, unsetField } from "./sheetEdit";
  import Self from "./SystemTreeEditor.svelte";

  // `root` is the resolved value at `basePath` on `doc` (the sheet passes the live system
  // object). Editing a leaf dispatches against `doc.id` at `basePath + subpointer`, reading
  // the REAL current value as the OCC pre-image. `readOnly` (advisory `canEdit` = false)
  // disables every control; the server stays authoritative.
  // INVARIANT: `doc` must be sourced from `ctx.documents` (the optimistic view), never
  // `ctx.store` (the rollback base) — otherwise every OCC pre-image read here is stale and
  // edits spuriously conflict [[render-from-optimistic-view]].
  let { doc, basePath, root, readOnly }: {
    /** The document edits dispatch against. Must come from the optimistic view (see the
     * INVARIANT above); the editor never reads or writes this itself, only forwards it. */
    doc: WireDocument;
    /** The JSON-pointer path from `doc`'s write root to `root`; a leaf edit targets
     * `basePath + "/" + key`. */
    basePath: string;
    /** The resolved value at `basePath` on `doc` — an object, array, or leaf to render. */
    root: unknown;
    /** Advisory `canEdit` result for `basePath`; `true` disables every control (the server
     * remains authoritative regardless). */
    readOnly: boolean;
  } = $props();

  const ctx = getAppContext();
  const t = ctx.t;

  const entries = $derived.by((): [string, unknown][] => {
    if (Array.isArray(root)) return root.map((v, i) => [String(i), v]);
    if (root !== null && typeof root === "object") return Object.entries(root as Record<string, unknown>);
    return [];
  });

  /**
   * Classify `v` for rendering: which input control (or recursive editor) a tree node gets.
   * `undefined` collapses to the same "null" kind as a real null: JSON never produces
   * `undefined`, so this branch is defensive-only, but without it a stray undefined would
   * fall through to "string" and render as an uneditable empty text input.
   * @param v - The value to classify.
   * @returns The rendering kind for `v`.
   * @example kindOf(42); // "number"
   */
  function kindOf(v: unknown): "string" | "number" | "boolean" | "object" | "array" | "null" {
    if (v === null || typeof v === "undefined") return "null";
    if (Array.isArray(v)) return "array";
    const tp = typeof v;
    if (tp === "number") return "number";
    if (tp === "boolean") return "boolean";
    if (tp === "object") return "object";
    return "string";
  }

  /**
   * Dispatch a leaf-value edit at `basePath + "/" + key`, reading the REAL current value
   * as the OCC pre-image (see the component-level invariant above).
   * @param key - The child key or array index within `root` being edited.
   * @param value - The new leaf value.
   * @example editLeaf("hp", 12);
   */
  function editLeaf(key: string, value: unknown): void {
    const path = `${basePath}/${key}`;
    setField(ctx, doc.id, path, getPointer(doc, path), value);
  }

  /**
   * Add a new object field seeded as an empty string. Array grow is not supported here
   * (`set_pointer` cannot extend arrays — the sheet writes the WHOLE array to grow it,
   * see {@link addArrayItem}). Generates a random key, retrying until it does not already
   * exist in `root` — always terminates in practice since `root` has finitely many keys.
   * @example addField();
   */
  function addField(): void {
    const existing = root !== null && typeof root === "object" && !Array.isArray(root) ? (root as Record<string, unknown>) : {};
    let key = crypto.randomUUID().slice(0, 8);
    while (key in existing) key = crypto.randomUUID().slice(0, 8);
    editLeaf(key, "");
  }

  /**
   * Remove `key` from `root`. Array-element removal stays whole-array replacement (neither
   * `set_pointer` nor `remove_pointer` can resize an array), rewriting the WHOLE array via
   * `setField`. Object-key removal is a narrow-OCC leaf remove (server `remove_pointer`):
   * only THIS key's pre-image is checked, so a concurrent edit to a sibling key does not
   * spuriously conflict as whole-container replacement would.
   * @param key - The child key (object) or index (array) to remove.
   * @example removeField("hp");
   */
  function removeField(key: string): void {
    if (Array.isArray(root)) {
      const next = (root as unknown[]).filter((_, i) => i !== Number(key));
      setField(ctx, doc.id, basePath, getPointer(doc, basePath), next);
    } else if (root !== null && typeof root === "object") {
      const path = `${basePath}/${key}`;
      unsetField(ctx, doc.id, path, getPointer(doc, path));
    }
  }

  /**
   * Append one element to the array at `root`, seeded matching the LAST existing element's
   * kind (an empty array defaults to string). `system` is opaque JSON with no schema layer
   * downstream to catch a heterogeneous array, so an always-string seed would permanently
   * type-pollute e.g. a `number[]` with a stray `""` that has no UI path back to a numeric
   * type. Rewrites the WHOLE array via `setField` (arrays cannot grow via `set_pointer`).
   * @example addArrayItem();
   */
  function addArrayItem(): void {
    const arr = root as unknown[];
    const lastKind = arr.length > 0 ? kindOf(arr[arr.length - 1]) : "string";
    const seed: unknown =
      lastKind === "number" ? 0 : lastKind === "boolean" ? false : lastKind === "array" ? [] : lastKind === "object" ? {} : "";
    const next = [...arr, seed];
    setField(ctx, doc.id, basePath, getPointer(doc, basePath), next);
  }
</script>

<ul class="tree">
  {#each entries as [key, value] (key)}
    <li class="node">
      <span class="key">{key}</span>
      {#if kindOf(value) === "string"}
        <input aria-label={key} value={value as string} disabled={readOnly}
          onchange={(e) => editLeaf(key, (e.currentTarget as HTMLInputElement).value)} />
      {:else if kindOf(value) === "number"}
        <input type="number" aria-label={key} value={value as number} disabled={readOnly}
          onchange={(e) => editLeaf(key, Number((e.currentTarget as HTMLInputElement).value))} />
      {:else if kindOf(value) === "boolean"}
        <input type="checkbox" aria-label={key} checked={value as boolean} disabled={readOnly}
          onchange={(e) => editLeaf(key, (e.currentTarget as HTMLInputElement).checked)} />
      {:else if kindOf(value) === "null"}
        <span class="null">{t("sheets.tree.null")}</span>
      {:else}
        <!-- object / array: recurse; the child edits against doc.id at the deeper path -->
        <Self {doc} basePath={`${basePath}/${key}`} root={value} {readOnly} />
      {/if}
      {#if !readOnly}
        <button type="button" class="remove" aria-label={t("sheets.tree.remove")} onclick={() => removeField(key)}>×</button>
      {/if}
    </li>
  {/each}
  {#if !readOnly}
    <li class="add">
      {#if Array.isArray(root)}
        <button type="button" onclick={addArrayItem}>{t("sheets.tree.addItem")}</button>
      {:else}
        <button type="button" onclick={addField}>{t("sheets.tree.addField")}</button>
      {/if}
    </li>
  {/if}
</ul>

<style lang="scss">
  .tree { list-style: none; margin: 0; padding-left: var(--space-2); display: flex; flex-direction: column; gap: var(--space-1); }
  .node { display: flex; align-items: center; gap: var(--space-1); flex-wrap: wrap; }
  .key { font-family: monospace; opacity: 0.8; }
  .null { opacity: 0.6; font-style: italic; }
  input:focus-visible, button:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }
  .remove { min-width: 24px; min-height: 24px; border: 1px solid var(--border); border-radius: var(--radius-1); background: var(--surface-raised); }
  @media (pointer: coarse) { .remove, .add button { min-height: 44px; min-width: 44px; } }
  .node input {
    @media (pointer: coarse) {
      min-height: var(--input-height-coarse);
    }
  }
</style>
