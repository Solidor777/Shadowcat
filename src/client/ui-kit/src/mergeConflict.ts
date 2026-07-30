import type { Conflict } from "@shadowcat/core";

/** One conflicted document/instance in the merge modal: its opaque group key
 * (document/instance UUID), display label, and the field-level conflicts the
 * user resolves mine-vs-theirs. Declared in a plain TS module (not the modal's
 * .svelte script) so tsc-based consumers (TypeDoc, d.ts emit) can resolve the
 * named export — .svelte modules expose only a default export to plain tsc. */
export type ConflictGroup = { key: string; label: string | null; conflicts: Conflict[] };
