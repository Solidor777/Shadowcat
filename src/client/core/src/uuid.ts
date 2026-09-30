// Secure-context-safe random UUIDs. `crypto.randomUUID` is exposed only in a secure context,
// and plain-HTTP LAN play (a non-loopback origin) is not one, while `crypto.getRandomValues` is
// available everywhere; this helper falls back to it so id generation never throws.

/** A random RFC 4122 version-4 UUID in lowercase hyphenated form. Uses `crypto.randomUUID` when
 * it is a function; otherwise builds the same shape from 16 `crypto.getRandomValues` bytes with
 * the version nibble set to 4 and the variant bits set to `10xx` (RFC 4122 section 4.4). The
 * result satisfies the server's document-id rule (a version 4 or version 7 UUID).
 * @returns A fresh UUID string, e.g. `"3b241101-e2bb-4255-8caf-4136c566a962"`.
 * @example
 * ```ts
 * import { randomUuid } from "@shadowcat/core";
 *
 * randomUuid().length; // 36
 * ```
 */
export function randomUuid(): string {
  if (typeof crypto.randomUUID === "function") return crypto.randomUUID();
  const b = crypto.getRandomValues(new Uint8Array(16));
  b[6] = (b[6] & 0x0f) | 0x40;
  b[8] = (b[8] & 0x3f) | 0x80;
  const h = Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`;
}
