<script lang="ts">
  import { getConfig, getMe } from "./entryApi";
  import Setup from "./views/Setup.svelte";
  import Login from "./views/Login.svelte";
  import WorldSelect from "./views/WorldSelect.svelte";

  // The pre-world flow is a self-contained package: the shell hands us callbacks
  // and we own the setup -> login -> world-select progression. Step is local state
  // (not the shell route), so a replacement entry package is a drop-in swap.
  let { onAuthenticated, onEnterWorld }: {
    /** Resolves true once the shell has the authenticated identity, false if the
     *  post-login identity fetch failed (entry then returns to login, not worlds). */
    onAuthenticated: () => boolean | Promise<boolean>;
    onEnterWorld: (worldId: string) => void;
  } = $props();

  type Step = "loading" | "setup" | "login" | "worlds";
  let step = $state<Step>("loading");

  /**
   * Decide which entry step to render on load: `setup` when the server has
   * no admin yet, `worlds` when an identity probe finds an authenticated
   * session, else `login`. A transient probe failure (network error, or a
   * non-2xx other than 401) falls to `login` rather than leaving the view on
   * `loading`.
   * @example
   * ```
   * // module-private; not part of the public API — invoked once at mount
   * decideStart();
   * ```
   */
  async function decideStart() {
    try {
      const cfg = await getConfig();
      if (!cfg.initialized) {
        step = "setup";
        return;
      }
      const me = await getMe();
      step = me ? "worlds" : "login";
    } catch {
      // A transient backend failure must not wedge entry on "loading"; fall to login.
      step = "login";
    }
  }
  decideStart();

  /**
   * Advance out of `login` once the shell has confirmed the session: `worlds`
   * on success, back to `login` on a failed identity fetch — a transient
   * failure must not strand the caller on a world-select it can't use.
   * @example
   * ```
   * // module-private; not part of the public API — passed to <Login onAuthed>
   * afterLogin();
   * ```
   */
  async function afterLogin() {
    // Awaiting onAuthenticated lets the shell fetch identity and apply saved
    // session state (locale) before world-select renders.
    step = (await onAuthenticated()) ? "worlds" : "login";
  }
</script>

{#if step === "setup"}
  <Setup onDone={() => (step = "login")} />
{:else if step === "login"}
  <Login onAuthed={afterLogin} />
{:else if step === "worlds"}
  <WorldSelect onEnter={onEnterWorld} />
{:else}
  <p class="connecting">Loading…</p>
{/if}
