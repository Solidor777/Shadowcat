import type { WireDocument, EffectEngine, EffectOrigin } from "@shadowcat/core";

/** One effect shown by `EffectsList`. */
export interface EffectEntry {
  /** A stable key: the effect's document id. */
  id: string;
  /** The effect's envelope display name, or the caller's fallback label when unnamed. */
  name: string;
  /** Whether the effect currently applies (`EffectEngine.active`). */
  active: boolean;
  /** The origin text: the origin label, else the resolved source document's name, else `null`. */
  origin: string | null;
}

/** The documents `listEffects` reads beyond the host: an id lookup for origin sources and the
 * candidate hosts scanned for effects anchored to a combatant. */
export interface EffectLookup {
  /** Resolves a document id to its readable document, or `undefined` when absent or unreadable.
   * @param id The document id.
   * @returns The document, or `undefined`. */
  get: (id: string) => WireDocument | undefined;
  /** Every readable document of a doc type.
   * @param docType The doc type to list.
   * @returns The matching documents. */
  query: (docType: string) => WireDocument[];
}

/**
 * Resolves an effect origin to display text: the label when present, else the name of the
 * source document when it still resolves, else `null`. A source that no longer resolves (a
 * deleted or unreadable document) yields `null`, never the raw id.
 * @param origin The effect's origin, or `null`/`undefined` when unrecorded.
 * @param lookup Resolves the source document.
 * @returns The text to show after the effect name, or `null` for none.
 * @example
 * ```ts
 * import { effectOriginText } from "@shadowcat/ui-kit";
 *
 * effectOriginText({ label: "Aldric" }, { get: () => undefined, query: () => [] }); // "Aldric"
 * ```
 */
export function effectOriginText(origin: EffectOrigin | null | undefined, lookup: EffectLookup): string | null {
  if (!origin) return null;
  if (origin.label) return origin.label;
  if (origin.source) return lookup.get(origin.source)?.name ?? null;
  return null;
}

/** An effect document paired with its decoded engine band. */
interface EffectDoc {
  /** The effect document. */
  doc: WireDocument;
  /** The document's decoded `EffectEngine` body. */
  engine: EffectEngine;
  /** The actor-shaped document the effect was read from; its embedded items resolve an origin
   * `source` that the store's top level does not hold. */
  bearer: WireDocument;
}

/**
 * Reads the effects embedded in one actor-shaped document: its own `effect` collection, then
 * the `effect` collection of each embedded item whose effect carries `transfer`. Mirrors the
 * server's host walk, so the list shows what the combat clock acts on.
 * @param actor An actor document, or a token's embedded actor copy.
 * @returns Each effect document with its decoded engine band.
 * @example
 * ```ts
 * // private function; not part of the public API — called by listEffects
 * const actor = undefined as unknown as WireDocument;
 * embeddedEffects(actor);
 * ```
 */
function embeddedEffects(actor: WireDocument): EffectDoc[] {
  const out: EffectDoc[] = [];
  for (const doc of actor.embedded?.effect ?? []) {
    if (doc.engine) out.push({ doc, engine: doc.engine as EffectEngine, bearer: actor });
  }
  for (const item of actor.embedded?.item ?? []) {
    for (const doc of item.embedded?.effect ?? []) {
      const engine = doc.engine as EffectEngine | undefined;
      if (engine?.transfer) out.push({ doc, engine, bearer: actor });
    }
  }
  return out;
}

/**
 * The effect-bearing actor document of a host: an actor itself, or a token's embedded actor
 * copy (`undefined` for a token with none).
 * @param host An actor or token document.
 * @returns The document whose `embedded` carries the effects.
 * @example
 * ```ts
 * // private function; not part of the public API — called by listEffects
 * const token = undefined as unknown as WireDocument;
 * effectBearer(token);
 * ```
 */
function effectBearer(host: WireDocument): WireDocument | undefined {
  return host.doc_type === "token" ? host.embedded?.actor?.[0] : host;
}

/**
 * Lists the effects on a host, each with its origin text. With `anchor`, also lists effects on
 * other actors and tokens whose clock is anchored to that combatant, so the list matches what
 * the combat clock ticks for it.
 * @param host The actor or token document whose effects to list.
 * @param lookup Resolves origin sources and supplies the anchor scan's candidates.
 * @param unnamed The label shown for an effect with no display name.
 * @param anchor A combatant id; effects elsewhere anchored to it are included.
 * @returns The entries in document order, host effects first.
 * @example
 * ```ts
 * import { listEffects } from "@shadowcat/ui-kit";
 * import type { WireDocument } from "@shadowcat/core";
 *
 * declare const actor: WireDocument;
 * listEffects(actor, { get: () => undefined, query: () => [] }, "Effect"); // [{ id, name, active, origin }]
 * ```
 */
export function listEffects(host: WireDocument, lookup: EffectLookup, unnamed: string, anchor?: string): EffectEntry[] {
  const entry = ({ doc, engine, bearer }: EffectDoc): EffectEntry => ({
    id: doc.id,
    name: doc.name ?? unnamed,
    active: engine.active,
    // An origin source may be an item embedded in the bearer, which `lookup.get` cannot see.
    origin: effectOriginText(engine.origin, {
      get: (id) => lookup.get(id) ?? bearer.embedded?.item?.find((item) => item.id === id),
      query: lookup.query,
    }),
  });
  const own = effectBearer(host);
  const entries = own ? embeddedEffects(own).filter(({ engine }) => !anchor || !engine.duration?.anchor || engine.duration.anchor === anchor).map(entry) : [];
  if (anchor) {
    for (const other of [...lookup.query("actor"), ...lookup.query("token")]) {
      if (other.id === host.id) continue;
      const bearer = effectBearer(other);
      if (!bearer) continue;
      for (const found of embeddedEffects(bearer)) {
        if (found.engine.duration?.anchor === anchor) entries.push(entry(found));
      }
    }
  }
  return entries;
}
