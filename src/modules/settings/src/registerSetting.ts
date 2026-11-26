import { SETTINGS_SECTION_CONTRACT, type ModuleContext } from "@shadowcat/core";
import SettingRow from "./SettingRow.svelte";

/** One selectable option of a `"select"` `SettingField`. */
export interface SettingSelectOption {
  /** The value passed to `set` when this option is chosen. */
  value: string;
  /** i18n key of the option's visible label. */
  labelKey: string;
}

/** A single settings row's value binding: a closed union of the three control kinds
 * `SettingRow` renders. `get`/`set` are the owning module's own storage; `registerSetting`
 * never persists a value itself. */
export type SettingField =
  | {
      /** Discriminant: a checkbox. */
      kind: "toggle";
      /** Reads the current value. */
      get: () => boolean;
      /** Writes a new value. */
      set: (value: boolean) => void;
    }
  | {
      /** Discriminant: a dropdown. */
      kind: "select";
      /** Reads the current value. */
      get: () => string;
      /** Writes a new value. */
      set: (value: string) => void;
      /** The selectable options, in display order. */
      options: SettingSelectOption[];
    }
  | {
      /** Discriminant: a single-line text input. */
      kind: "text";
      /** Reads the current value. */
      get: () => string;
      /** Writes a new value. */
      set: (value: string) => void;
      /** Placeholder text, already translated; omitted for none. */
      placeholder?: string;
    };

/** Options for `registerSetting`. */
export interface RegisterSettingOptions {
  /** The contribution id; unique across the contributing module's settings sections. */
  id: string;
  /** i18n key of the section heading and of the row's label. */
  labelKey: string;
  /** The row's value binding. */
  field: SettingField;
}

/**
 * Contributes one single-field settings section: a `SETTINGS_SECTION_CONTRACT` contribution
 * rendering `SettingRow` for `field`, under a `labelKey` heading. A strict subset of the raw
 * contribution shape; a module needing a richer control contributes its own component instead.
 * @param ctx The registering module's context; only `contributions` is read.
 * @param opts The contribution id, label key and value binding.
 * @returns A dispose function that removes the contribution.
 * @example
 * ```ts
 * import type { ModuleContext } from "@shadowcat/core";
 * import { registerSetting } from "@shadowcat/module-settings";
 *
 * declare const ctx: Pick<ModuleContext, "contributions">;
 * let enabled = false;
 * const dispose = registerSetting(ctx, {
 *   id: "example:enabled",
 *   labelKey: "example.enabled",
 *   field: { kind: "toggle", get: () => enabled, set: (v) => (enabled = v) },
 * });
 * dispose();
 * ```
 */
export function registerSetting(ctx: Pick<ModuleContext, "contributions">, opts: RegisterSettingOptions): () => void {
  return ctx.contributions.contribute({
    id: opts.id,
    contract: SETTINGS_SECTION_CONTRACT,
    component: SettingRow,
    props: { field: opts.field, labelKey: opts.labelKey },
    settingsSection: { labelKey: opts.labelKey },
  });
}
