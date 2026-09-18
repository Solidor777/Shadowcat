import type { AppContext } from "@shadowcat/ui-kit";

/** The slice of `AppContext` this function reads. See `performanceSeam.ts`'s
 * `PerformanceReader` doc for why the caller's already-resolved `ctx` is threaded through
 * rather than calling `getAppContext()` internally. */
type AudioReader = Pick<AppContext, "audio">;

/**
 * Plays the dice-clatter one-shot through the world's audio mixer at throw time.
 * @param ctx The caller's already-resolved `AppContext`.
 * @param sound The `dice-settings.sound` asset id, or `null` for silent.
 * @example
 * ```ts
 * import { playThrowSound } from "@shadowcat/module-dice-3d";
 * import { getAppContext } from "@shadowcat/ui-kit";
 *
 * playThrowSound(getAppContext(), null);
 * ```
 */
export function playThrowSound(ctx: AudioReader, sound: string | null): void {
  if (!sound) return;
  ctx.audio.playOneShot(sound, { channel: "sfx" });
}
