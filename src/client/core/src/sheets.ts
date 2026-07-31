// Sheet registry resolution + document-reference write-site resolution (M12c).
// Pure: no Svelte, no panel-manager, no AppContext. `openDocument` (ui-kit) and the
// generic sheet components consume these. Fail-closed everywhere — a dangling or
// raw reference resolves to `null`, never a throw.
import type { WireDocument } from "./wire";
import type { ReadableDocuments } from "./store";
import type { ContributionRegistry } from "./contributions";

/** Contract family a sheet provider registers under, keyed by target doc_type. */
export const SHEET_CONTRACT_PREFIX = "shadowcat.sheet:";
/** The always-registered generic fallback contract (any doc_type, priority -Infinity). */
export const SHEET_FALLBACK_CONTRACT = "shadowcat.sheet:*";

/** Builds the `shadowcat.sheet:<doc_type>` contract id a sheet provider
 * registers under for `docType`.
 * @param docType The document's `doc_type`.
 * @returns The contract id sheet providers for `docType` register under.
 * @example
 * ```ts
 * import { sheetContract } from "@shadowcat/core";
 *
 * sheetContract("actor"); // "shadowcat.sheet:actor"
 * ```
 */
export function sheetContract(docType: string): string {
  return SHEET_CONTRACT_PREFIX + docType;
}

/** What `ctx.openDocument` addresses. `docId` targets a top-level document, optionally
 * one embedded child via `embeddedPath` (`/embedded/<collection>/<index>`, ONE level —
 * an actor's inventory item, etc.). `tokenId` targets a placed token, resolved to its
 * linked actor or its embedded actor copy per §5.2. */
export type SheetRef = { docId: string; embeddedPath?: string } | { tokenId: string };

/** The resolved open target: the panel id (dedups re-opens), the document to READ for
 * display + registry doc_type pick, and the write site every field-path Update is
 * addressed to (`writeDocId` + `writePrefix` — always the TOP-LEVEL doc + the JSON
 * pointer of its writable `system` body). */
export interface SheetTarget {
  panelId: string;
  doc: WireDocument;
  writeDocId: string;
  writePrefix: string;
}

/** Reads a ONE-level embedded child (`/embedded/<coll>/<idx>`); null on any malformed
 * path or out-of-range index. Not exported — folded into `resolveDocRef`'s public surface.
 * @param top The parent document to read the embedded child from.
 * @param embeddedPath A `/embedded/<collection>/<index>` JSON pointer.
 * @returns The embedded child document, or `null` if `embeddedPath` is malformed
 * or names a missing collection/index.
 * @example
 * ```
 * // internal helper; not part of the public API (see resolveDocRef for the public entry point)
 * readEmbeddedChild(top, "/embedded/actor/0");
 * ```
 */
function readEmbeddedChild(top: WireDocument, embeddedPath: string): WireDocument | null {
  const m = /^\/embedded\/([^/]+)\/(\d+)$/.exec(embeddedPath);
  if (!m) return null;
  const child = top.embedded?.[m[1]]?.[Number(m[2])];
  return child ?? null;
}

/** Resolves a `SheetRef` to its open target: the panel id to dedup re-opens by,
 * the document to READ for display, and the write site (`writeDocId` +
 * `writePrefix`) every field-path Update should be addressed to. Fail-closed —
 * every dangling, raw, or malformed reference returns `null`; this function
 * never throws.
 * @param ref The reference to resolve (see `SheetRef`'s per-variant resolution
 * rules — linked token → shared actor, instanced token → embedded copy,
 * top-level doc, or embedded child).
 * @param store The document store to resolve `ref` against.
 * @returns The resolved open target, or `null` if `ref` doesn't resolve to an
 * openable document.
 * @example
 * ```ts
 * import { resolveDocRef, DocumentStore } from "@shadowcat/core";
 *
 * const store = new DocumentStore();
 * const target = resolveDocRef({ docId: "00000000-0000-0000-0000-000000000001" }, store);
 * ```
 */
export function resolveDocRef(ref: SheetRef, store: ReadableDocuments): SheetTarget | null {
  // Fail-closed for untyped runtime callers — the `in` operator below throws on primitives.
  if (!ref || typeof ref !== "object") return null;
  if ("tokenId" in ref) {
    const token = store.get(ref.tokenId);
    if (!token) return null;
    // `actor_id` is engine-owned (TokenEngine); the sheet itself still reads/writes the
    // actor's `/system` (game-system data — untouched by the three-band re-root).
    const eng = token.engine as { actor_id?: string | null } | undefined;
    // Linked: write the SHARED actor doc's /system (mirrors conditionTarget). A dangling
    // link (actor gone) fails closed — never a phantom sheet over a missing doc.
    if (eng?.actor_id) {
      const actor = store.get(eng.actor_id);
      if (!actor) return null;
      return { panelId: "sheet:" + actor.id, doc: actor, writeDocId: actor.id, writePrefix: "/system" };
    }
    // Instanced: write the TOKEN doc's embedded copy at /embedded/actor/0/system.
    const embedded = token.embedded?.actor?.[0];
    if (!embedded) return null; // raw/actorless token — nothing to open
    // Self-describing panelId: the bare "sheet:" + token.id is string-identical to a
    // top-level docId panelId, so a persisted-layout reverse-parse (docId + embeddedPath
    // recovered from the id string) would rebind this panel to the token's own /system
    // instead of its embedded actor. Encoding the embedded path in the id lets it round-trip
    // through the embedded-child branch below.
    return {
      panelId: "sheet:" + token.id + "/embedded/actor/0",
      doc: embedded,
      writeDocId: token.id,
      writePrefix: "/embedded/actor/0/system",
    };
  }
  const top = store.get(ref.docId);
  if (!top) return null;
  if (ref.embeddedPath) {
    const child = readEmbeddedChild(top, ref.embeddedPath);
    if (!child) return null;
    return { panelId: `sheet:${top.id}${ref.embeddedPath}`, doc: child, writeDocId: top.id, writePrefix: ref.embeddedPath + "/system" };
  }
  return { panelId: "sheet:" + top.id, doc: top, writeDocId: top.id, writePrefix: "/system" };
}

/** Resolve the sheet COMPONENT for a document: doc_type providers plus the generic
 * fallback, filtered by each provider's `match`, highest `priority` wins, ties broken by
 * lexicographically lowest registering module id (the M11d-3 deterministic-singleton
 * precedent). `-Infinity` keeps the fallback below every real provider. Null only when
 * nothing (not even a fallback) is registered.
 * @param registry The contribution registry to search for sheet providers.
 * @param doc The document to pick a sheet component for.
 * @returns The winning provider's opaque `component` handle, or `null` if no
 * provider (not even the fallback) is registered.
 * @example
 * ```ts
 * import { pickSheet, ContributionRegistry } from "@shadowcat/core";
 * import type { WireDocument } from "@shadowcat/core";
 *
 * const registry = new ContributionRegistry();
 * declare const doc: WireDocument;
 * const component = pickSheet(registry, doc);
 * ```
 */
export function pickSheet(registry: ContributionRegistry, doc: WireDocument): unknown | null {
  const seen = new Set<unknown>();
  const candidates = [
    ...registry.entriesFor(sheetContract(doc.doc_type)),
    // A doc_type of literally "*" collides with the fallback contract by construction —
    // both entriesFor calls would return the same entries. Dedupe by Contribution object
    // identity (entriesFor wraps entries fresh per call, but wraps the SAME contribution),
    // never by id string: the registry does not guarantee id uniqueness across contracts,
    // so an id-keyed dedupe could drop a legitimate distinct provider.
    ...registry.entriesFor(SHEET_FALLBACK_CONTRACT),
  ]
    .filter((e) => e.contribution.sheet && (!e.contribution.sheet.match || e.contribution.sheet.match(doc)))
    .filter((e) => (seen.has(e.contribution) ? false : (seen.add(e.contribution), true)));
  if (candidates.length === 0) return null;
  candidates.sort((a, b) => {
    const pa = a.contribution.sheet!.priority, pb = b.contribution.sheet!.priority;
    // Explicit relational comparison (not subtraction): -Infinity - -Infinity is NaN, which
    // a subtraction comparator treats as "equal" but Array.sort's NaN handling is
    // unspecified — relational comparison always falls through correctly to the tie-break.
    if (pb !== pa) return pb > pa ? 1 : -1;
    return (a.module ?? a.contribution.id).localeCompare(b.module ?? b.contribution.id);
  });
  return candidates[0].contribution.component;
}

/** Heuristic: does a string look like dice notation (`NdM`, optional `+K`/`-K`)? The
 * server owns real parsing; the client only decides whether to SHOW a roll affordance.
 * Trimmed, case-insensitive.
 * @param s The string to test.
 * @returns `true` if `s` looks like dice notation.
 * @example
 * ```ts
 * import { isDiceNotation } from "@shadowcat/core";
 *
 * isDiceNotation("2d6+3"); // true
 * isDiceNotation("hello"); // false
 * ```
 */
export function isDiceNotation(s: string): boolean {
  return /^\s*\d*d\d+(\s*[+-]\s*\d+)?\s*$/i.test(s);
}
