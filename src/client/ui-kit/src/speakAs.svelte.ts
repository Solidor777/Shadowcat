/**
 * The session's sticky "speak as this actor" selection — the actor a roll or message is attributed to and, since dice
 * references resolve server-side against the send's actor binding, the document a roll's references
 * read. Lifted out of the composer so every roll-producing surface (the composer, a chat card's
 * roll buttons) resolves the SAME selection: a statted roll button clicked by a player resolves
 * against THEIR current speak-as, not the button author's stats.
 *
 * A stable instance held by the shell and shared via AppContext. Sibling of `SpeakAsToken`: same
 * stable-instance/mutate-in-place shape; differs in being STICKY (a selection that persists until
 * changed, like the composer's `<select>` always was), not a one-shot. Like its siblings, it does
 * not prune when the referenced actor is deleted — readers resolve against the current store and
 * the server re-validates every send's attribution at ingest.
 */
export class SpeakAs {
  /** Backing store for {@link SpeakAs.actorId}. */
  #actorId = $state("");

  /** The sticky speak-as actor id, or `""` when posting as yourself (no actor bound).
   * @returns The sticky actor id, or `""`. */
  get actorId(): string {
    return this.#actorId;
  }
  /** Sets (or clears, with `""`) the sticky speak-as actor.
   * @param id - The actor id to speak as, or `""` to clear.
   * @example speakAs.actorId = "actor-1";
   */
  set actorId(id: string) {
    this.#actorId = id;
  }
}
