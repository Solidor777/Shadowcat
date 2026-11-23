<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext, ConditionTogglePalette } from "@shadowcat/ui-kit";
  import { randomUuid, buildUpdate, type Condition, type ConditionFx, type ConditionRegistryEngine, type WireDocument } from "@shadowcat/core";

  const ctx = getAppContext();
  const t = ctx.t;

  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));
  const registry = $derived.by((): WireDocument | undefined => {
    subscribe();
    return ctx.documents.query("condition-registry")[0];
  });
  const conditionEntries = $derived.by((): [string, Condition][] => {
    const sys = registry?.engine as ConditionRegistryEngine | undefined;
    return Object.entries(sys?.conditions ?? {});
  });

  // The condition registry is server-seeded (world creation / world join) with the engine's
  // default emoji set; this panel only reads and edits it. The default content stays
  // replaceable per the `conditions` module's modding contract: a game-system module can
  // supply its own registry editor instead.

  /** GM registry editor: patches one or more fields of a condition entry, dispatching one
   * `update` op per changed field. `old` reads the RAW currently-stored value (never a
   * resolved/defaulted one) — the server's field-level optimistic-concurrency check
   * (`apply_intent`) rejects an `Update` whose `old` doesn't match the actual stored value,
   * so a hardcoded `old: null` would only be valid for the field's first write.
   * @param id The condition's registry key.
   * @param patch The fields to change (name and/or icon).
   * @example
   * ```
   * // private function; not part of the public API — invoked from the GM editor row's
   * // name/icon inputs
   * update("prone", { name: "Prone" });
   * ```
   */
  function update(id: string, patch: Partial<Condition>): void {
    if (!registry) return;
    // `old` must be the field's REAL current stored value (or null when genuinely absent): the
    // server's apply_intent enforces field-level OCC (actual != change.old -> Conflict), so a
    // hardcoded `old: null` is only valid once and is rejected on every subsequent edit once the
    // field holds a non-null value (mirrors GameSettingsPanel's `set` helper fix).
    const eng = registry.engine as ConditionRegistryEngine;
    const current = eng.conditions[id] as Partial<Condition> | undefined;
    for (const [k, v] of Object.entries(patch)) {
      const old = current?.[k as keyof Condition] ?? null;
      ctx.dispatchIntent([buildUpdate(registry.id, [{ path: `/engine/conditions/${id}/${k}`, old, value: v }])]);
    }
  }
  /** GM registry editor: sets or clears ONE field of a condition's authored fx, dispatching a
   * whole-fx `update` op (the fx payload replaces wholesale — writing per-sub-field paths would
   * strand a cleared sibling key in the stored object). `old` reads the RAW currently-stored fx
   * (never a resolved/defaulted one), the same OCC convention as `update`. A fully-cleared fx
   * writes `null` (no effect), keeping the stored payload minimal.
   * @param id The condition's registry key.
   * @param key The fx field to change (`tint`/`desaturate`/`highlight`).
   * @param value The new field value, or `undefined` to clear the field.
   * @example
   * ```
   * // private function; not part of the public API — invoked from the GM editor row's fx inputs
   * setFx("poisoned", "tint", "#66ff66");
   * ```
   */
  function setFx(id: string, key: keyof ConditionFx, value: string | boolean | undefined): void {
    if (!registry) return;
    const eng = registry.engine as ConditionRegistryEngine;
    const old = eng.conditions[id]?.fx ?? null;
    const next: ConditionFx = { ...(old ?? {}) };
    if (value === undefined) delete next[key];
    else if (key === "desaturate") next.desaturate = value === true;
    else next[key] = String(value);
    const empty = !next.tint && !next.desaturate && !next.highlight;
    ctx.dispatchIntent([buildUpdate(registry.id, [{ path: `/engine/conditions/${id}/fx`, old, value: empty ? null : next }])]);
  }
  /** GM registry editor: appends a new condition entry under a fresh random id, with a
   * placeholder name/icon for the GM to rename in place.
   * @example
   * ```
   * // private function; not part of the public API — invoked from the "Add" button
   * add();
   * ```
   */
  function add(): void {
    if (!registry) return;
    const id = randomUuid();
    const c: Condition = { name: "New condition", icon: "⭐" };
    ctx.dispatchIntent([buildUpdate(registry.id, [{ path: `/engine/conditions/${id}`, old: null, value: c }])]);
  }
  /** GM registry editor: deletes a condition entry from the registry map.
   * @param id The condition's registry key to remove.
   * @example
   * ```
   * // private function; not part of the public API — invoked from each row's remove button
   * remove("prone");
   * ```
   */
  function remove(id: string): void {
    const sys = registry?.engine as ConditionRegistryEngine | undefined;
    if (!registry || !sys) return;
    const next = { ...sys.conditions };
    delete next[id];
    ctx.dispatchIntent([buildUpdate(registry.id, [{ path: "/engine/conditions", old: sys.conditions, value: next }])]);
  }

</script>

<section class="conditions">
  <h3>{t("conditions.title")}</h3>

  <ConditionTogglePalette />

  {#if ctx.role === "gm"}
    <ul class="list">
      {#each conditionEntries as [id, c] (id)}
        <li>
          <span class="glyph">{c.icon}</span>
          <input aria-label={t("conditions.name")} value={c.name} onchange={(e) => update(id, { name: e.currentTarget.value })} />
          <input aria-label={t("conditions.icon")} value={c.icon} maxlength="4" onchange={(e) => update(id, { icon: e.currentTarget.value })} />
          <span class="fx">
            <input type="color" aria-label={t("conditions.tint")} title={t("conditions.tint")} value={c.fx?.tint ?? "#ff0000"} onchange={(e) => setFx(id, "tint", e.currentTarget.value)} />
            {#if c.fx?.tint}
              <button type="button" aria-label={t("conditions.clearTint")} onclick={() => setFx(id, "tint", undefined)}>×</button>
            {/if}
            <label class="desaturate">
              <input type="checkbox" checked={!!c.fx?.desaturate} onchange={(e) => setFx(id, "desaturate", e.currentTarget.checked ? true : undefined)} />
              {t("conditions.desaturate")}
            </label>
            <input type="color" aria-label={t("conditions.highlight")} title={t("conditions.highlight")} value={c.fx?.highlight ?? "#ffffff"} onchange={(e) => setFx(id, "highlight", e.currentTarget.value)} />
            {#if c.fx?.highlight}
              <button type="button" aria-label={t("conditions.clearHighlight")} onclick={() => setFx(id, "highlight", undefined)}>×</button>
            {/if}
          </span>
          <button type="button" onclick={() => remove(id)}>{t("conditions.remove")}</button>
        </li>
      {/each}
    </ul>
    <button type="button" onclick={add}>{t("conditions.add")}</button>
  {/if}
</section>

<style lang="scss">
  .conditions {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-1);
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .list li {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    flex-wrap: wrap;
  }
  .fx {
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }
  .fx input[type="color"] {
    width: 32px;
    min-width: 32px;
    padding: 0;
  }
  .desaturate {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    font-size: 0.85em;
    color: var(--text-secondary);
  }
  .glyph {
    font-size: 1.1em;
    flex: 0 0 auto;
  }
  input,
  button {
    min-height: 32px;
  }
</style>
