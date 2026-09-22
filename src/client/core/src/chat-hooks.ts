// The chat composer's client-only pre-send interception seam: a hook a module registers via
// its own `ModuleContext.hooks.on` to transform or veto an outgoing message's text BEFORE
// `Composer.svelte` builds the `SendMessage` frame, so a game-system module can implement its
// own slash commands without a server-side change. The resulting text (or nothing, if
// cancelled) still travels the unchanged server pipeline — no client-side command execution.
import type { HookBus } from "./hooks";

/** Payload for `chat:composer-send`. A `mutate`-kind hook: dispatch threads this object
 * through every listener in priority order, each listener's return value becoming the next
 * listener's input, so ONE dispatch lets a listener both rewrite `text` and set `cancel` in a
 * single round trip rather than needing a second `cancel`-kind dispatch to veto. */
export interface ChatComposerSendPayload {
  /** The trimmed draft text about to be sent. A listener returns a modified copy to rewrite it
   * (e.g. expanding a system-specific slash command into ordinary text). */
  text: string;
  /** Set `true` to veto the send entirely; `Composer.svelte` checks this after every listener
   * has run and, if set, never calls `AppContext.chat.send`. Defaults to `false`. */
  cancel: boolean;
}

declare module "./hooks" {
  /** Declaration-merged with `./hooks`'s own `CoreHooks` to add the composer's
   * pre-send interception hook. */
  interface CoreHooks {
    /** Fires once per composer send attempt, before the frame is built — see
     * `ChatComposerSendPayload`. */
    "chat:composer-send": ChatComposerSendPayload;
  }
}

/** Semver of the `chat:composer-send` hook payload contract declared above. */
export const CHAT_COMPOSER_HOOK_VERSION = "1.0.0";

/** Declares `chat:composer-send` on `hooks` as a `"mutate"`-kind hook at
 * {@link CHAT_COMPOSER_HOOK_VERSION}. Idempotent to call more than once at the same version (a
 * second `WorldSession` construction in the same process, e.g. under test).
 * @param hooks The bus to declare against.
 * @example
 * ```ts
 * import { HookBus, silentLogger } from "@shadowcat/core";
 * import { defineChatComposerHooks } from "@shadowcat/core";
 *
 * const hooks = new HookBus(silentLogger);
 * defineChatComposerHooks(hooks);
 * ```
 */
export function defineChatComposerHooks(hooks: HookBus): void {
  hooks.defineHook("chat:composer-send", { version: CHAT_COMPOSER_HOOK_VERSION, kind: "mutate" });
}
