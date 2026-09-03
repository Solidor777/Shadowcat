<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { parseFormula } from "@shadowcat/formula";
  import type { WireDocument, ResourceRegistryEngine, Resource, ResourceBinding, Formula } from "@shadowcat/core";

  const ctx = getAppContext();

  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));
  const registry = $derived.by((): WireDocument | undefined => {
    subscribe();
    return ctx.documents.query("resource-registry")[0];
  });
  const entries = $derived.by((): [string, Resource][] => {
    const eng = registry?.engine as ResourceRegistryEngine | undefined;
    return Object.entries(eng?.resources ?? {}).sort((a, b) => a[1].order - b[1].order);
  });

  /** Coerces a text input into a `Formula`: a finite numeric literal becomes a number, else a
   * valid formula string is kept trimmed, else `null` (the caller shows the inline error and
   * skips the write). Mirrors `CombatSettings.onLifecycleInput`'s coercion rule.
   * @param text The raw input value.
   * @returns The coerced `Formula`, or `null` when `text` is neither a number nor a valid formula. */
  function coerceFormula(text: string): Formula | null {
    const trimmed = text.trim();
    const n = Number(trimmed);
    if (trimmed !== "" && Number.isFinite(n)) return n;
    const parsed = parseFormula(trimmed);
    if ("error" in parsed) return null;
    return trimmed;
  }

  let errors = $state<Record<string, string | null>>({});

  /** Writes ONE field of a resource entry (name, order, or a binding sub-path), reading the raw
   * currently-stored value as the OCC pre-image.
   * @param key The resource's registry key.
   * @param path The field's JSON-pointer path suffix under `/engine/resources/<key>`.
   * @param old The field's real current stored value.
   * @param value The new value to write. */
  function writeField(key: string, path: string, old: unknown, value: unknown): void {
    if (!registry) return;
    ctx.dispatchIntent([{ op: "update", doc_id: registry.id, changes: [{ path: `/engine/resources/${key}${path}`, old: old ?? null, new: value }] }]);
  }

  function onFormulaInput(key: string, errKey: string, path: string, old: unknown, text: string): void {
    const coerced = coerceFormula(text);
    if (coerced === null) {
      errors[errKey] = ctx.t("gameSettings.resources.invalid", { detail: text });
      return;
    }
    errors[errKey] = null;
    writeField(key, path, old, coerced);
  }

  /** Kind switch: one whole-object write at the entry's `binding` path, replacing the binding
   * with the new kind's defaults — `name`/`order` are untouched since they live at sibling
   * paths, not inside the binding object being replaced.
   * @param key The resource's registry key.
   * @param oldBinding The raw currently-stored binding (the OCC pre-image).
   * @param kind The newly-selected binding kind. */
  function switchKind(key: string, oldBinding: ResourceBinding, kind: ResourceBinding["kind"]): void {
    const next: ResourceBinding =
      kind === "mirror"
        ? { kind: "mirror", value: 0 }
        : { kind: "tracked", max: 0, recover: { turn_start: 0, turn_end: 0, round_start: 0, round_end: 0 } };
    writeField(key, "/binding", oldBinding, next);
  }

  let newKey = $state("");
  let addError = $state<string | null>(null);
  const KEY_SHAPE = /^[a-z][a-z0-9_-]*$/;

  /** Validates the new-entry key (shape + uniqueness against the current registry) and, if
   * valid, dispatches the whole-entry write with `old: null`.
   * @example
   * ```
   * // private function; not part of the public API — invoked from the "Add" button
   * add();
   * ```
   */
  function add(): void {
    if (!registry) return;
    const key = newKey.trim();
    if (!KEY_SHAPE.test(key)) {
      addError = ctx.t("gameSettings.resources.keyShape");
      return;
    }
    const eng = registry.engine as ResourceRegistryEngine;
    if (key in eng.resources) {
      addError = ctx.t("gameSettings.resources.keyTaken");
      return;
    }
    addError = null;
    const order = Object.keys(eng.resources).length;
    const entry: Resource = { name: key, order, binding: { kind: "mirror", value: 0 } };
    ctx.dispatchIntent([{ op: "update", doc_id: registry.id, changes: [{ path: `/engine/resources/${key}`, old: null, new: entry }] }]);
    newKey = "";
  }

  /** Rewrites the whole `resources` map without the removed key (the `ConditionsPanel.remove`
   * shape — a per-key removal has no `remove: true` leaf form here since the map itself, not a
   * leaf, is the unit the OCC pre-image is read from).
   * @param key The resource's registry key to remove.
   * @example
   * ```
   * // private function; not part of the public API — invoked from each row's remove button
   * remove("gold");
   * ```
   */
  function remove(key: string): void {
    if (!registry) return;
    const eng = registry.engine as ResourceRegistryEngine;
    const next = { ...eng.resources };
    delete next[key];
    ctx.dispatchIntent([{ op: "update", doc_id: registry.id, changes: [{ path: "/engine/resources", old: eng.resources, new: next }] }]);
  }
</script>

{#if ctx.role === "gm" && registry}
  <fieldset>
    <legend>{ctx.t("gameSettings.resources.title")}</legend>

    {#each entries as [key, entry] (key)}
      <fieldset>
        <legend>{key}</legend>
        <label>
          {ctx.t("gameSettings.resources.name")}
          <input type="text" aria-label={"gameSettings.resources.name-" + key} value={entry.name}
            onchange={(e) => writeField(key, "/name", entry.name, (e.currentTarget as HTMLInputElement).value)} />
        </label>
        <label>
          {ctx.t("gameSettings.resources.order")}
          <input type="number" step="1" aria-label={"gameSettings.resources.order-" + key} value={entry.order}
            onchange={(e) => writeField(key, "/order", entry.order, Number((e.currentTarget as HTMLInputElement).value))} />
        </label>
        <label>
          {ctx.t("gameSettings.resources.kind")}
          <select aria-label={"gameSettings.resources.kind-" + key} value={entry.binding.kind}
            onchange={(e) => switchKind(key, entry.binding, (e.currentTarget as HTMLSelectElement).value as ResourceBinding["kind"])}>
            <option value="mirror">{ctx.t("gameSettings.resources.mirror")}</option>
            <option value="tracked">{ctx.t("gameSettings.resources.tracked")}</option>
          </select>
        </label>

        {#if entry.binding.kind === "mirror"}
          <label>
            {ctx.t("gameSettings.resources.value")}
            <input type="text" aria-label={"gameSettings.resources.value-" + key} value={String(entry.binding.value)}
              onchange={(e) => onFormulaInput(key, key + ":value", "/binding/value", entry.binding.kind === "mirror" ? entry.binding.value : null, (e.currentTarget as HTMLInputElement).value)} />
          </label>
          {#if errors[key + ":value"]}<p class="error">{errors[key + ":value"]}</p>{/if}
        {:else}
          <label>
            {ctx.t("gameSettings.resources.max")}
            <input type="text" aria-label={"gameSettings.resources.max-" + key} value={String(entry.binding.max)}
              onchange={(e) => onFormulaInput(key, key + ":max", "/binding/max", entry.binding.kind === "tracked" ? entry.binding.max : null, (e.currentTarget as HTMLInputElement).value)} />
          </label>
          {#if errors[key + ":max"]}<p class="error">{errors[key + ":max"]}</p>{/if}
          {#each [["turnStart", "turn_start"], ["turnEnd", "turn_end"], ["roundStart", "round_start"], ["roundEnd", "round_end"]] as [labelKey, field] (field)}
            <label>
              {ctx.t("gameSettings.resources." + labelKey)}
              <input type="text" aria-label={"gameSettings.resources." + labelKey + "-" + key}
                value={String(entry.binding.kind === "tracked" ? entry.binding.recover[field as keyof typeof entry.binding.recover] : "")}
                onchange={(e) => onFormulaInput(key, key + ":" + field, "/binding/recover/" + field, entry.binding.kind === "tracked" ? entry.binding.recover[field as keyof typeof entry.binding.recover] : null, (e.currentTarget as HTMLInputElement).value)} />
            </label>
            {#if errors[key + ":" + field]}<p class="error">{errors[key + ":" + field]}</p>{/if}
          {/each}
        {/if}

        <button type="button" aria-label={"gameSettings.resources.remove-" + key} onclick={() => remove(key)}>{ctx.t("gameSettings.resources.remove")}</button>
      </fieldset>
    {/each}

    <label>
      {ctx.t("gameSettings.resources.key")}
      <input type="text" aria-label="gameSettings.resources.key" value={newKey}
        onchange={(e) => (newKey = (e.currentTarget as HTMLInputElement).value)} />
    </label>
    <button type="button" onclick={add}>{ctx.t("gameSettings.resources.add")}</button>
    {#if addError}<p class="error">{addError}</p>{/if}
  </fieldset>
{/if}
