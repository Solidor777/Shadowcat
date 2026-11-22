/** Most entries the composer's sent-text history keeps per world; the oldest is evicted first. */
export const CHAT_HISTORY_LIMIT = 50;

/** Largest serialized size, in UTF-8 bytes of `JSON.stringify(history)`, the composer history may
 * reach. The account's whole `ui_state` blob is capped server-side (64 KiB across every world and
 * key), and a rejected write would also block the panel layout and read markers riding the same
 * patch, so the history is bounded by size as well as by entry count. */
export const CHAT_HISTORY_BYTE_BUDGET = 16 * 1024;

const encoder = new TextEncoder();

/** Serialized size of a history list as the `ui_state` write carries it.
 * @param history The list to measure.
 * @returns The UTF-8 byte length of its JSON form.
 * @example
 * ```ts
 * const n = serializedBytes(["hi"]);
 * ```
 */
function serializedBytes(history: readonly string[]): number {
  return encoder.encode(JSON.stringify(history)).length;
}

/** Appends `text` as the newest entry of a composer history, then evicts the oldest entries until
 * the list holds at most `CHAT_HISTORY_LIMIT` entries and fits `CHAT_HISTORY_BYTE_BUDGET`. An entry
 * that alone exceeds the byte budget is not recorded.
 * @param history The stored history, oldest first.
 * @param text The raw typed text just sent.
 * @returns A new list, oldest first; `history` is not mutated.
 * @example
 * ```ts
 * import { pushChatHistory } from "@shadowcat/core";
 *
 * const next = pushChatHistory(["/roll 1d20"], "hello");
 * ```
 */
export function pushChatHistory(history: readonly string[], text: string): string[] {
  if (serializedBytes([text]) > CHAT_HISTORY_BYTE_BUDGET) return [...history];
  const next = [...history, text];
  while (next.length > CHAT_HISTORY_LIMIT) next.shift();
  while (serializedBytes(next) > CHAT_HISTORY_BYTE_BUDGET) next.shift();
  return next;
}

/** Reads the opaque stored blob as a composer history; the shell stores whatever the account's
 * `ui_state` holds, so a malformed value yields an empty history rather than an error.
 * @param blob The stored value, or `null` when none was saved.
 * @returns The string members of the blob when it is an array, otherwise an empty list.
 * @example
 * ```ts
 * import { parseChatHistory } from "@shadowcat/core";
 *
 * const history = parseChatHistory(null);
 * ```
 */
export function parseChatHistory(blob: unknown): string[] {
  if (!Array.isArray(blob)) return [];
  return blob.filter((entry): entry is string => typeof entry === "string");
}
