import type { Conflict } from "@shadowcat/core";

/** One conflicted document/instance in the merge modal: its opaque group key
 * (document/instance UUID), display label, and the field-level conflicts the
 * user resolves mine-vs-theirs. Declared in a plain TS module (not the modal's
 * .svelte script) so tsc-based consumers (TypeDoc, d.ts emit) can resolve the
 * named export — .svelte modules expose only a default export to plain tsc. */
export type ConflictGroup = {
  /** Opaque group key (the document/instance UUID); the modal's resolver map is keyed by this. */
  key: string;
  /** Display label for the group header, or `null` when the caller has no name to show
   * (single-group pull sessions omit it). */
  label: string | null;
  /** The field-level conflicts within this document/instance the user resolves mine-vs-theirs. */
  conflicts: Conflict[];
};
