/** The `localStorage` key holding the per-device chat-bubbles toggle (the dice-3d and ducking
 * mirrors' precedent for a display preference that is not a world policy). */
export const CHAT_BUBBLES_STORAGE_KEY = "shadowcat.chatBubbles";

/**
 * Whether this device draws chat bubbles over speaking tokens. A display preference only: the
 * chat panel shows every message regardless. Reactive (`$state`), so the overlay hides at once
 * when it is switched off; persisted to `localStorage` as `"0"`/`"1"`. A missing, unreadable or
 * unwritable store leaves the default (on) in effect for the session.
 */
export class ChatBubblesPreference {
  /** Backing store for {@link ChatBubblesPreference.enabled}. */
  #enabled = $state(true);

  /**
   * Reads the persisted value from `storage`.
   * @param storage The store to read and write; defaults to `localStorage` (injectable for tests).
   * @example
   * ```ts
   * import { ChatBubblesPreference } from "@shadowcat/module-chat-bubbles";
   *
   * const pref = new ChatBubblesPreference(localStorage);
   * ```
   */
  constructor(private readonly storage: Pick<Storage, "getItem" | "setItem"> | null = typeof localStorage === "undefined" ? null : localStorage) {
    try {
      if (this.storage?.getItem(CHAT_BUBBLES_STORAGE_KEY) === "0") this.#enabled = false;
    } catch {
      // An unreadable store keeps the default.
    }
  }

  /** Whether bubbles are drawn.
   * @returns `true` when bubbles are on (the default). */
  get enabled(): boolean {
    return this.#enabled;
  }

  /** Switch bubbles on or off and persist the choice.
   * @param value The new state.
   * @example pref.set(false);
   */
  set(value: boolean): void {
    this.#enabled = value;
    try {
      this.storage?.setItem(CHAT_BUBBLES_STORAGE_KEY, value ? "1" : "0");
    } catch {
      // An unwritable store keeps the choice for this session only.
    }
  }
}
