<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext } from "@shadowcat/ui-kit";
  import { resolveSettingProvenance, type WireDocument, type WorldSettingsEngine, type CombatDefaults, type SettingPath, type ResourceRegistryEngine } from "@shadowcat/core";
  import { parseFormula } from "@shadowcat/formula";

  /** CombatSettings props. */
  interface Props {
    /** The world-settings document, when it exists. */
    ws: WireDocument | undefined;
    /** `ws.engine`, cast. */
    wsys: WorldSettingsEngine | undefined;
    /** The panel's single-field JSON-pointer write helper. */
    set: (docId: string, path: string, old: unknown, value: unknown) => void;
    /** The panel's world-defaults provenance resolver (scene always `undefined`). */
    prov: (path: SettingPath) => {
      /** The resolved value at the winning tier. */
      value: unknown;
      /** Which tier the value resolved from. */
      source: "engine" | "system" | "world" | "scene";
    };
    /** The scene currently selected in the per-scene section, for the effective-rules summary. */
    scene: WireDocument | undefined;
  }
  const { ws, wsys, set, prov, scene }: Props = $props();

  const ctx = getAppContext();
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));

  const registryDoc = $derived.by((): WireDocument | undefined => {
    subscribe();
    return ctx.documents.query("resource-registry")[0];
  });
  const registryKeys = $derived.by((): string[] => {
    const eng = registryDoc?.engine as ResourceRegistryEngine | undefined;
    return Object.keys(eng?.resources ?? {});
  });

  const LIFECYCLE_LEAVES = ["onCombatEnd", "onTurnEnd", "onAdvance"] as const;
  const INTERPRETATION = ["per_cell", "spaces"] as const;
  const ENFORCEMENT = ["none", "warn", "hard"] as const;
  const TURN_CONTROL = ["owner_may_end", "gm_only"] as const;

  /** Writes the WHOLE `/engine/combat` object: `set_pointer` cannot create a missing `/engine/
   * combat` from a leaf sub-path, so every combat-leaf write replaces the object outright, with
   * the RAW stored object (or `null`) as the OCC pre-image.
   * @param next The replacement `CombatDefaults` object, or `null` to clear it entirely.
   * @example
   * ```
   * // private function; not part of the public API — invoked from every leaf write below
   * writeCombat({ enforcement: "warn" });
   * ```
   */
  function writeCombat(next: CombatDefaults | null): void {
    if (!ws) return;
    set(ws.id, "/engine/combat", wsys?.combat ?? null, next);
  }

  /** Whether `next` carries no authored leaves at all (every field absent) — a combat object
   * collapsed to nothing is written as `null`, never an empty object, so provenance correctly
   * reports the layer beneath falling through.
   * @param next The candidate object.
   * @returns Whether it is empty.
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
    const next: CombatDefaults = { ...(wsys?.combat ?? {}), [key]: value };
    writeCombat(next);
  }

  /** Clears one `CombatDefaults` leaf (falls through to system/engine), collapsing the whole
   * object to `null` when nothing is left overridden.
   * @param key The leaf being cleared.
   * @example
   * ```
   * // private function; not part of the public API — invoked from every "Inherit" reset
   * leafRemove("enforcement");
   * ```
   */
  function leafRemove(key: keyof CombatDefaults): void {
    const next: CombatDefaults = { ...(wsys?.combat ?? {}) };
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
    const lifecycle = { ...(wsys?.combat?.effectLifecycle ?? {}), [leaf]: value };
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
    const lifecycle = { ...(wsys?.combat?.effectLifecycle ?? {}) };
    delete lifecycle[leaf];
    if (Object.keys(lifecycle).length === 0) leafRemove("effectLifecycle");
    else leafSet("effectLifecycle", lifecycle);
  }

  /** Coerces a lifecycle text input: a finite numeric literal writes a number, else a valid
   * formula writes the trimmed string, else the inline error is shown and nothing is written.
   * Blank input removes the leaf (inherit).
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

  /** Writes the world-tier `movementResource` selection: `"__inherit"` clears the override,
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

  const EFFECTIVE_PATHS: SettingPath[] = [
    "combat.movementResource", "combat.interpretation", "combat.enforcement", "combat.turnControl",
    "combat.effectCleanup", "combat.rewindRestore", "combat.forwardRestore",
    "combat.effectLifecycle.onCombatEnd", "combat.effectLifecycle.onTurnEnd", "combat.effectLifecycle.onAdvance",
  ];

  /** The effective-rules table's rows: one `resolveSettingProvenance` resolution per
   * `EFFECTIVE_PATHS` entry, re-derived whenever `ctx.documents` changes. `subscribe()` bridges
   * `ctx.documents`'s plain-callback reactivity into this `$derived` — without it the table
   * freezes at first render (at world creation, before any combat setting is written) and never
   * reflects a later edit, the same reactivity bug `GameSettingsPanel` itself avoids. */
  const effectiveRows = $derived.by((): {
    /** The `SettingPath` this row resolves. */
    path: SettingPath;
    /** The resolved value at the winning tier. */
    value: unknown;
    /** Which tier the value resolved from. */
    source: string;
  }[] => {
    subscribe();
    return EFFECTIVE_PATHS.map((path) => {
      const r = resolveSettingProvenance(ctx.documents, scene, path);
      return { path, value: r.value, source: r.source };
    });
  });
</script>

<fieldset>
  <legend>{ctx.t("gameSettings.combat.title")}</legend>

  <label>
    {ctx.t("gameSettings.combat.movementResource")}
    <select aria-label="gameSettings.combat.movementResource" value={prov("combat.movementResource").value ?? "__inherit"}
      onchange={(e) => onMovementResourceChange((e.currentTarget as HTMLSelectElement).value)}>
      <option value="__inherit">{ctx.t("gameSettings.inherit")}</option>
      <option value="__none">{ctx.t("gameSettings.combat.none")}</option>
      {#each registryKeys as key (key)}<option value={key}>{key}</option>{/each}
    </select>
  </label>
  <p data-testid="provenance:combat.movementResource">{ctx.t("gameSettings.source." + prov("combat.movementResource").source)}</p>
  {#if prov("combat.movementResource").source === "world"}
    <button type="button" onclick={() => leafRemove("movementResource")}>{ctx.t("gameSettings.resetToSystem")}</button>
  {/if}

  <label>
    {ctx.t("gameSettings.combat.interpretation")}
    <select aria-label="gameSettings.combat.interpretation" value={wsys?.combat?.interpretation ?? "__inherit"}
      onchange={(e) => {
        const v = (e.currentTarget as HTMLSelectElement).value;
        if (v === "__inherit") leafRemove("interpretation");
        else leafSet("interpretation", v as CombatDefaults["interpretation"]);
      }}>
      <option value="__inherit">{ctx.t("gameSettings.inherit")}</option>
      {#each INTERPRETATION as v}<option value={v}>{v}</option>{/each}
    </select>
  </label>
  <p data-testid="provenance:combat.interpretation">{ctx.t("gameSettings.source." + prov("combat.interpretation").source)}</p>
  {#if prov("combat.interpretation").source === "world"}
    <button type="button" onclick={() => leafRemove("interpretation")}>{ctx.t("gameSettings.resetToSystem")}</button>
  {/if}

  <label>
    {ctx.t("gameSettings.combat.enforcement")}
    <select aria-label="gameSettings.combat.enforcement" value={wsys?.combat?.enforcement ?? "__inherit"}
      onchange={(e) => {
        const v = (e.currentTarget as HTMLSelectElement).value;
        if (v === "__inherit") leafRemove("enforcement");
        else leafSet("enforcement", v as CombatDefaults["enforcement"]);
      }}>
      <option value="__inherit">{ctx.t("gameSettings.inherit")}</option>
      {#each ENFORCEMENT as v}<option value={v}>{v}</option>{/each}
    </select>
  </label>
  <p data-testid="provenance:combat.enforcement">{ctx.t("gameSettings.source." + prov("combat.enforcement").source)}</p>
  {#if prov("combat.enforcement").source === "world"}
    <button type="button" onclick={() => leafRemove("enforcement")}>{ctx.t("gameSettings.resetToSystem")}</button>
  {/if}

  <label>
    {ctx.t("gameSettings.combat.turnControl")}
    <select aria-label="gameSettings.combat.turnControl" value={wsys?.combat?.turnControl ?? "__inherit"}
      onchange={(e) => {
        const v = (e.currentTarget as HTMLSelectElement).value;
        if (v === "__inherit") leafRemove("turnControl");
        else leafSet("turnControl", v as CombatDefaults["turnControl"]);
      }}>
      <option value="__inherit">{ctx.t("gameSettings.inherit")}</option>
      {#each TURN_CONTROL as v}<option value={v}>{v}</option>{/each}
    </select>
  </label>
  <p data-testid="provenance:combat.turnControl">{ctx.t("gameSettings.source." + prov("combat.turnControl").source)}</p>
  {#if prov("combat.turnControl").source === "world"}
    <button type="button" onclick={() => leafRemove("turnControl")}>{ctx.t("gameSettings.resetToSystem")}</button>
  {/if}

  {#each [["effectCleanup", "gameSettings.combat.effectCleanup"], ["rewindRestore", "gameSettings.combat.rewindRestore"], ["forwardRestore", "gameSettings.combat.forwardRestore"]] as [key, labelKey] (key)}
    {@const k = key as "effectCleanup" | "rewindRestore" | "forwardRestore"}
    <label>
      {ctx.t(labelKey)}
      <select aria-label={labelKey} value={wsys?.combat?.[k] === undefined || wsys?.combat?.[k] === null ? "__inherit" : String(wsys.combat[k])}
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
    <p data-testid={"provenance:combat." + key}>{ctx.t("gameSettings.source." + prov(("combat." + key) as SettingPath).source)}</p>
    {#if prov(("combat." + key) as SettingPath).source === "world"}
      <button type="button" onclick={() => leafRemove(k)}>{ctx.t("gameSettings.resetToSystem")}</button>
    {/if}
  {/each}

  <fieldset>
    <legend>{ctx.t("gameSettings.combat.lifecycle")}</legend>
    {#each LIFECYCLE_LEAVES as leaf (leaf)}
      <label>
        {ctx.t("gameSettings.combat." + leaf)}
        <input type="text" aria-label={"gameSettings.combat." + leaf}
          value={wsys?.combat?.effectLifecycle?.[leaf] ?? ""}
          onchange={(e) => onLifecycleInput(leaf, (e.currentTarget as HTMLInputElement).value)} />
      </label>
      {#if lifecycleErrors[leaf]}
        <p class="error">{ctx.t("gameSettings.resources.invalid", { detail: lifecycleErrors[leaf] ?? "" })}</p>
      {/if}
      <p data-testid={"provenance:combat.effectLifecycle." + leaf}>{ctx.t("gameSettings.source." + prov(("combat.effectLifecycle." + leaf) as SettingPath).source)}</p>
    {/each}
  </fieldset>

  <table data-testid="combat-tracker-effective-rules">
    <caption>{ctx.t("gameSettings.combat.effective")}</caption>
    <tbody>
      {#each effectiveRows as r (r.path)}
        <tr>
          <td>{r.path}</td>
          <td data-testid={"gameSettings:combat-effective-" + r.path}>{JSON.stringify(r.value)}</td>
          <td>{ctx.t("gameSettings.source." + r.source)}</td>
        </tr>
      {/each}
    </tbody>
  </table>
</fieldset>
