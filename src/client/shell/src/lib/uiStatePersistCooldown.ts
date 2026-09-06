/** Leading-edge debounce cooldown for the ui-state persist scheduler (`schedulePersist`).
 *
 * This is a dependency-free leaf module so the browser suite can read the same value the shell
 * uses. `sessionState.svelte.ts` cannot serve that role: its import chain reaches Svelte
 * components, which the Playwright spec loader cannot parse.
 *
 * INVARIANT: any wait that must outlast this cooldown derives from this constant instead of
 * restating it. A second spelling of the number erodes silently, because only a machine slow
 * enough to expose the gap would ever fail on it.
 * @example
 * ```
 * const settleMs = COOLDOWN_MS + 200;
 * ```
 */
export const COOLDOWN_MS = 500;
