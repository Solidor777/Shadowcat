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

  async function afterLogin() {
    // Let the shell fetch identity + apply saved session state (locale) before
    // world-select renders, mirroring the pre-split boot order. A failed identity
    // fetch returns to login (the old afterAuth `me ? "worlds" : "login"` recovery),
    // so a transient failure can't strand the user on a world-select they can't use.
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
