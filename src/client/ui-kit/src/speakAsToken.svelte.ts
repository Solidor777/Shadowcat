/**
 * The token instance a scene-tools affordance has picked to speak as for the composer's NEXT
 * message send — a one-shot pending selection, distinct from the composer's own sticky actor
 * `<select>`. A stable instance held by the shell and shared via AppContext: `ToolRail` sets it
 * (the "speak as this token" button), the composer consumes it on send.
 *
 * Sibling of `SceneSelection`: same stable-instance/mutate-in-place shape (`$state` +
 * `select`), and likewise does not prune when the referenced token is later deleted/deselected
 * — the composer resolves against the current document store and handles a miss itself.
 * Diverges in offering `consume()`, since the pending value here targets only the next message
 * sent and is read-once by design, not a persistent selection like `SceneSelection`'s.
 */
export class SpeakAsToken {
  /** Backing store for {@link SpeakAsToken.tokenId}. */
  #tokenId = $state<string | null>(null);

  /** The pending speak-as token id, or `null` when nothing is pending.
   * @returns The pending token id, or `null`. */
  get tokenId(): string | null {
    return this.#tokenId;
  }

  /** Set (or clear, with `null`) the pending speak-as token.
   * @param id - The token id to target, or `null` to clear.
   * @example speakAsToken.select("tok-1");
   */
  select(id: string | null): void {
    this.#tokenId = id;
  }

  /** Reads the pending token id and clears it in the same step — the composer's one-shot
   * consume-on-send contract, targeting only the next message sent: a rejected/aborted send
   * does not restore the consumed value, mirroring how the composer's draft text is also
   * cleared optimistically before a send's outcome is known.
   * @returns The pending token id that was set, or `null` if nothing was pending.
   * @example speakAsToken.consume(); // returns "tok-1" and clears back to null
   */
  consume(): string | null {
    const id = this.#tokenId;
    this.#tokenId = null;
    return id;
  }
}
