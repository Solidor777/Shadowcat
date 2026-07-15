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
 * path or out-of-range index. */
function readEmbeddedChild(top: WireDocument, embeddedPath: string): WireDocument | null {
  const m = /^\/embedded\/([^/]+)\/(\d+)$/.exec(embeddedPath);
  if (!m) return null;
  const child = top.embedded?.[m[1]]?.[Number(m[2])];
  return child ?? null;
}

export function resolveDocRef(ref: SheetRef, store: ReadableDocuments): SheetTarget | null {
  // Fail-closed for untyped runtime callers — the `in` operator below throws on primitives.
  if (!ref || typeof ref !== "object") return null;
  if ("tokenId" in ref) {
    const token = store.get(ref.tokenId);
    if (!token) return null;
    const sys = token.system as { actor_id?: string | null } | undefined;
    // Linked: write the SHARED actor doc's /system (mirrors conditionTarget). A dangling
    // link (actor gone) fails closed — never a phantom sheet over a missing doc.
    if (sys?.actor_id) {
      const actor = store.get(sys.actor_id);
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
 * nothing (not even a fallback) is registered. */
export function pickSheet(registry: ContributionRegistry, doc: WireDocument): unknown | null {
  const seen = new Set<string>();
  const candidates = [
    ...registry.entriesFor(sheetContract(doc.doc_type)),
    // A doc_type of literally "*" collides with the fallback contract by construction —
    // both entriesFor calls would return the same entries; dedupe below by contribution id.
    ...registry.entriesFor(SHEET_FALLBACK_CONTRACT),
  ]
    .filter((e) => e.contribution.sheet && (!e.contribution.sheet.match || e.contribution.sheet.match(doc)))
    .filter((e) => (seen.has(e.contribution.id) ? false : (seen.add(e.contribution.id), true)));
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
 * Trimmed, case-insensitive. */
export function isDiceNotation(s: string): boolean {
  return /^\s*\d*d\d+(\s*[+-]\s*\d+)?\s*$/i.test(s);
}
