/** A human-readable byte size, one decimal place, binary (1024-based) units. Shared by every
 * surface presenting a byte count: the asset browser's metadata panel and the admin trash
 * listing both format through this. Accepts a `bigint` because i64-sourced wire values arrive
 * as one; it is normalized through `Number()` first.
 * @param n - Byte count.
 * @returns The formatted size, such as `"512 B"`, `"2.0 KiB"` or `"3.4 MiB"`.
 * @example
 * ```ts
 * import { fmtBytes } from "@shadowcat/core";
 *
 * fmtBytes(2048); // "2.0 KiB"
 * ```
 */
export function fmtBytes(n: number | bigint): string {
  const v = Number(n);
  if (v < 1024) return `${v} B`;
  if (v < 1024 * 1024) return `${(v / 1024).toFixed(1)} KiB`;
  return `${(v / (1024 * 1024)).toFixed(1)} MiB`;
}
