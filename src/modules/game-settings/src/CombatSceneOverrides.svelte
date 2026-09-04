<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import { resolveSettingProvenance, type WireDocument, type SceneEngine, type CombatDefaults, type SettingPath, type ResourceRegistryEngine } from "@shadowcat/core";
  import { parseFormula } from "@shadowcat/formula";

  /** CombatSceneOverrides props. */
  interface Props {
    /** The currently selected scene document. */
    scene: WireDocument;
    /** `scene.engine`, cast. */
    ssys: SceneEngine | undefined;
    /** The panel's per-scene single-field JSON-pointer write helper. */
    setScene: (path: string, old: unknown, value: unknown) => void;
  }
  const { scene, ssys, setScene }: Props = $props();

  const ctx = getAppContext();

  const registryKeys = $derived.by((): string[] => {
    const doc = ctx.documents.query("resource-registry")[0];
    const eng = doc?.engine as ResourceRegistryEngine | undefined;
    return Object.keys(eng?.resources ?? {});
  });

  const LIFECYCLE_LEAVES = ["onCombatEnd", "onTurnEnd", "onAdvance"] as const;
  const INTERPRETATION = ["per_cell", "spaces"] as const;
  const ENFORCEMENT = ["none", "warn", "hard"] as const;
  const TURN_CONTROL = ["owner_may_end", "gm_only"] as const;

  /** Reads the `combat.<leaf>` chain resolved AT this scene — `resolveSettingProvenance`
   * already re-roots to the scene tier when a real scene (not `undefined`) is passed, so this
   * editor needs no separate world-tier resolver like `CombatSettings`' `prov` prop.
   * @param path The `SettingPath` leaf to resolve.
   * @returns The resolved value and which tier it came from.
   * @example
   * ```
   * // private function; not part of the public API — invoked from every provenance readout
   * prov("combat.enforcement");
   * ```
   */
  function prov(path: SettingPath): {
    /** The resolved value at the winning tier. */
    value: unknown;
    /** Which tier the value resolved from. */
    source: "engine" | "system" | "world" | "scene";
  } {
    return resolveSettingProvenance(ctx.documents, scene, path);
  }

  /** Writes the WHOLE `/engine/combat` object on the SELECTED SCENE document — same
   * whole-object-replace rule as `CombatSettings.writeCombat`, applied to the scene doc instead
   * of the world-settings doc, since `set_pointer` cannot create a missing `/engine/combat`
   * parent from a leaf sub-path.
   * @param next The replacement `CombatDefaults` object, or `null` to clear it entirely.
   * @example
   * ```
   * // private function; not part of the public API — invoked from every leaf write below
   * writeCombat({ enforcement: "warn" });
   * ```
   */
  function writeCombat(next: CombatDefaults | null): void {
    setScene("/engine/combat", ssys?.combat ?? null, next);
  }

  /** Whether a `CombatDefaults` object has no overrides left, and can collapse to `null`.
   * @param next The candidate `CombatDefaults` object.
   * @returns `true` when `next` carries no keys.
   * @example
   * ```
   * // private function; not part of the public API — invoked from leafRemove
   * isEmptyCombat({});
   * ```
   */
  function isEmptyCombat(next: CombatDefaults): boolean {
    return Object.keys(next).length === 0;
  }

  /** Sets one `CombatDefaults` leaf, preserving every other authored override.
   * @param key The leaf being set.
   * @param value The leaf's new value.
   * @example
   * ```
   * // private function; not part of the public API — invoked from every scalar leaf control
   * leafSet("enforcement", "warn");
   * ```
   */
  function leafSet<K extends keyof CombatDefaults>(key: K, value: CombatDefaults[K]): void {
    const next: CombatDefaults = { ...(ssys?.combat ?? {}), [key]: value };
    writeCombat(next);
  }

  /** Clears one `CombatDefaults` leaf (falls through to world/system/engine), collapsing the
   * whole object to `null` when nothing is left overridden.
   * @param key The leaf being cleared.
   * @example
   * ```
   * // private function; not part of the public API — invoked from every "Inherit" reset
   * leafRemove("enforcement");
   * ```
   */
  function leafRemove(key: keyof CombatDefaults): void {
    const next: CombatDefaults = { ...(ssys?.combat ?? {}) };
    delete next[key];
    writeCombat(isEmptyCombat(next) ? null : next);
  }

  /** Sets one `effectLifecycle` sub-leaf, preserving every other authored lifecycle leaf.
   * @param leaf Which lifecycle field is being set.
   * @param value The leaf's new value (a numeric literal or a formula string).
   * @example
   * ```
   * // private function; not part of the public API — invoked from onLifecycleInput
   * lifecycleSet("onCombatEnd", 1);
   * ```
   */
  function lifecycleSet(leaf: (typeof LIFECYCLE_LEAVES)[number], value: number | string): void {
    const lifecycle = { ...(ssys?.combat?.effectLifecycle ?? {}), [leaf]: value };
    leafSet("effectLifecycle", lifecycle);
  }

  /** Clears one `effectLifecycle` sub-leaf, removing the whole `effectLifecycle` object when no
   * sub-leaf is left overridden.
   * @param leaf Which lifecycle field is being cleared.
   * @example
   * ```
   * // private function; not part of the public API — invoked from onLifecycleInput on blank text
   * lifecycleRemove("onCombatEnd");
   * ```
   */
  function lifecycleRemove(leaf: (typeof LIFECYCLE_LEAVES)[number]): void {
    const lifecycle = { ...(ssys?.combat?.effectLifecycle ?? {}) };
    delete lifecycle[leaf];
    if (Object.keys(lifecycle).length === 0) leafRemove("effectLifecycle");
    else leafSet("effectLifecycle", lifecycle);
  }

  /** Coerces a lifecycle text input the same way `CombatSettings.onLifecycleInput` does: a
   * finite numeric literal writes a number, else a valid formula writes the trimmed string,
   * else the inline error is shown and nothing is written. Blank input removes the leaf
   * (falls through to world/system/engine).
   * @param leaf Which lifecycle field the input edits.
   * @param text The raw input value.
   * @example
   * ```
   * // private function; not part of the public API — invoked from a lifecycle input's onchange
   * onLifecycleInput("onCombatEnd", "1");
   * ```
   */
  function onLifecycleInput(leaf: (typeof LIFECYCLE_LEAVES)[number], text: string): void {
    const trimmed = text.trim();
    if (trimmed === "") {
      lifecycleErrors[leaf] = null;
      lifecycleRemove(leaf);
      return;
    }
    const n = Number(trimmed);
    if (Number.isFinite(n) && trimmed !== "") {
      lifecycleErrors[leaf] = null;
      lifecycleSet(leaf, n);
      return;
    }
    const parsed = parseFormula(trimmed);
    if ("error" in parsed) {
      lifecycleErrors[leaf] = parsed.detail;
      return;
    }
    lifecycleErrors[leaf] = null;
    lifecycleSet(leaf, trimmed);
  }

  let lifecycleErrors = $state<Record<string, string | null>>({ onCombatEnd: null, onTurnEnd: null, onAdvance: null });

  /** Writes the scene-tier `movementResource` selection: `"__inherit"` clears the override,
   * `"__none"` explicitly clears the inherited resource, else the chosen registry key is set.
   * @param value The `<select>`'s chosen option value.
   * @example
   * ```
   * // private function; not part of the public API — invoked from the movement-resource select
   * onMovementResourceChange("movement");
   * ```
   */
  function onMovementResourceChange(value: string): void {
    if (value === "__inherit") leafRemove("movementResource");
    else if (value === "__none") leafSet("movementResource", null);
    else leafSet("movementResource", value);
  }
</script>

<fieldset>
  <legend>{ctx.t("gameSettings.combat.scene.title")}</legend>

  <label>
    {ctx.t("gameSettings.combat.movementResource")}
    <select aria-label="gameSettings.combat.scene.movementResource" value={prov("combat.movementResource").value ?? "__inherit"}
      onchange={(e) => onMovementResourceChange((e.currentTarget as HTMLSelectElement).value)}>
      <option value="__inherit">{ctx.t("gameSettings.inherit")}</option>
      <option value="__none">{ctx.t("gameSettings.combat.none")}</option>
      {#each registryKeys as key (key)}<option value={key}>{key}</option>{/each}
    </select>
  </label>
  <p data-testid="provenance:combat.scene.movementResource">{ctx.t("gameSettings.source." + prov("combat.movementResource").source)}</p>

  <label>
    {ctx.t("gameSettings.combat.interpretation")}
    <select aria-label="gameSettings.combat.scene.interpretation" value={ssys?.combat?.interpretation ?? "__inherit"}
      onchange={(e) => {
        const v = (e.currentTarget as HTMLSelectElement).value;
        if (v === "__inherit") leafRemove("interpretation");
        else leafSet("interpretation", v as CombatDefaults["interpretation"]);
      }}>
      <option value="__inherit">{ctx.t("gameSettings.inherit")}</option>
      {#each INTERPRETATION as v}<option value={v}>{v}</option>{/each}
    </select>
  </label>
  <p data-testid="provenance:combat.scene.interpretation">{ctx.t("gameSettings.source." + prov("combat.interpretation").source)}</p>

  <label>
    {ctx.t("gameSettings.combat.enforcement")}
    <select aria-label="gameSettings.combat.scene.enforcement" value={ssys?.combat?.enforcement ?? "__inherit"}
      onchange={(e) => {
        const v = (e.currentTarget as HTMLSelectElement).value;
        if (v === "__inherit") leafRemove("enforcement");
        else leafSet("enforcement", v as CombatDefaults["enforcement"]);
      }}>
      <option value="__inherit">{ctx.t("gameSettings.inherit")}</option>
      {#each ENFORCEMENT as v}<option value={v}>{v}</option>{/each}
    </select>
  </label>
  <p data-testid="provenance:combat.scene.enforcement">{ctx.t("gameSettings.source." + prov("combat.enforcement").source)}</p>

  <label>
    {ctx.t("gameSettings.combat.turnControl")}
    <select aria-label="gameSettings.combat.scene.turnControl" value={ssys?.combat?.turnControl ?? "__inherit"}
      onchange={(e) => {
        const v = (e.currentTarget as HTMLSelectElement).value;
        if (v === "__inherit") leafRemove("turnControl");
        else leafSet("turnControl", v as CombatDefaults["turnControl"]);
      }}>
      <option value="__inherit">{ctx.t("gameSettings.inherit")}</option>
      {#each TURN_CONTROL as v}<option value={v}>{v}</option>{/each}
    </select>
  </label>
  <p data-testid="provenance:combat.scene.turnControl">{ctx.t("gameSettings.source." + prov("combat.turnControl").source)}</p>

  {#each [["effectCleanup", "gameSettings.combat.effectCleanup"], ["rewindRestore", "gameSettings.combat.rewindRestore"], ["forwardRestore", "gameSettings.combat.forwardRestore"]] as [key, labelKey] (key)}
    {@const k = key as "effectCleanup" | "rewindRestore" | "forwardRestore"}
    <label>
      {ctx.t(labelKey)}
      <select aria-label={"gameSettings.combat.scene." + key} value={ssys?.combat?.[k] === undefined || ssys?.combat?.[k] === null ? "__inherit" : String(ssys.combat[k])}
        onchange={(e) => {
          const v = (e.currentTarget as HTMLSelectElement).value;
          if (v === "__inherit") leafRemove(k);
          else leafSet(k, v === "true");
        }}>
        <option value="__inherit">{ctx.t("gameSettings.inherit")}</option>
        <option value="true">{ctx.t("gameSettings.combat.on")}</option>
        <option value="false">{ctx.t("gameSettings.combat.off")}</option>
      </select>
    </label>
    <p data-testid={"provenance:combat.scene." + key}>{ctx.t("gameSettings.source." + prov(("combat." + key) as SettingPath).source)}</p>
  {/each}

  <fieldset>
    <legend>{ctx.t("gameSettings.combat.lifecycle")}</legend>
    {#each LIFECYCLE_LEAVES as leaf (leaf)}
      <label>
        {ctx.t("gameSettings.combat." + leaf)}
        <input type="text" aria-label={"gameSettings.combat.scene." + leaf}
          value={ssys?.combat?.effectLifecycle?.[leaf] ?? ""}
          onchange={(e) => onLifecycleInput(leaf, (e.currentTarget as HTMLInputElement).value)} />
      </label>
      {#if lifecycleErrors[leaf]}
        <p class="error">{ctx.t("gameSettings.resources.invalid", { detail: lifecycleErrors[leaf] ?? "" })}</p>
      {/if}
      <p data-testid={"provenance:combat.scene.effectLifecycle." + leaf}>{ctx.t("gameSettings.source." + prov(("combat.effectLifecycle." + leaf) as SettingPath).source)}</p>
    {/each}
  </fieldset>
</fieldset>
