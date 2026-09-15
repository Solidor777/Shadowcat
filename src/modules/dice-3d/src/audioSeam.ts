/**
 * Rewired to call
 * `getAppContext().audio.playOneShot(sound, { channel: "sfx" })` once `AudioApi`
 * exists. A no-op until then — `dice-settings.sound` has nowhere to play through yet.
 * @param sound The `dice-settings.sound` asset id, or `null` for silent.
 * @example
 * ```ts
 * import { playThrowSound } from "@shadowcat/module-dice-3d";
 *
 * playThrowSound(null);
 * ```
 */
export function playThrowSound(sound: string | null): void {
  void sound; // no-op until `AudioApi` exists (see this function's doc)
}
