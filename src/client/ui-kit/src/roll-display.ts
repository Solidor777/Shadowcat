import { numericBounds, type DieRecord } from "@shadowcat/core";

/** Which end of its face range a die's NATURAL face landed on. */
export type NatExtreme = "min" | "max";

/** Whether a die rolled the lowest or highest natural face of its `Numeric` face range.
 * Reads only `kind` and `natural`, never the crit flags, so a Total-mode roll (which carries no
 * crit configuration) highlights a natural minimum or maximum too. A `Faces` die has no ranked
 * face order and a record without `kind` (stored before the field existed) has no known range,
 * so neither ever reports an extreme. The one declaration every roll renderer calls.
 * @param record The die's post-pipeline record.
 * @returns `"min"` or `"max"` when `record.natural` equals that bound of its `Numeric` range,
 * else `null`. A one-sided range (`min === max`) reports `"max"`.
 * @example
 * ```ts
 * import { natExtreme } from "./roll-display";
 *
 * natExtreme({ natural: 20, kind: { Numeric: { min: 1, max: 20 } } } as DieRecord); // "max"
 * ```
 */
export function natExtreme(record: Pick<DieRecord, "natural" | "kind">): NatExtreme | null {
  if (!record.kind) return null;
  const bounds = numericBounds(record.kind);
  if (!bounds) return null;
  if (record.natural === bounds.max) return "max";
  if (record.natural === bounds.min) return "min";
  return null;
}
