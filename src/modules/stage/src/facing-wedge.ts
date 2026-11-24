import { ownerFloorApplies, type ReadableDocuments, type WireDocument, type WorldRole } from "@shadowcat/core";

/** Whether a viewer may see a token's facing wedge: the GM, or the token's effective owner
 * (`ownerFloorApplies`: the direct owner, or the owner floor reached through the linked actor).
 * Rotation permission is not the test: a world whose capability defaults let every player turn
 * any token would otherwise show every creature's facing to every player.
 * @param role The viewer's world role.
 * @param selfId The viewer's user id.
 * @param doc The token document.
 * @param documents The document view the owner floor is resolved against.
 * @returns `true` when the wedge may be drawn for this viewer.
 * @example
 * ```
 * declare const token: WireDocument;
 * declare const documents: ReadableDocuments;
 * facingWedgeVisibleTo("player", "user-1", token, documents);
 * ```
 */
export function facingWedgeVisibleTo(
  role: WorldRole,
  selfId: string,
  doc: WireDocument,
  documents: ReadableDocuments,
): boolean {
  return role === "gm" || ownerFloorApplies(doc, selfId, documents);
}
