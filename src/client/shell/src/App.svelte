<script lang="ts">
  import { webSocketConnect } from "@shadowcat/core";
  import { Entry } from "@shadowcat/module-entry";
  import { getMe, listWorlds, type Me } from "./lib/api";
  import { loadSessionState, setLastWorld, flushOnUnload } from "./lib/sessionState.svelte";
  import { currentRoute, navigate } from "./lib/route.svelte";
  import { coreUi } from "./modules/core-ui/index";
  import { sceneTools } from "./modules/scene-tools/index";
  import { WorldSession } from "./lib/worldSession.svelte";
  import Table from "./lib/Table.svelte";

  let me = $state<Me | null>(null);
  let booted = $state(false);
  let session = $state<WorldSession | null>(null);

  async function boot() {
    try {
      me = await getMe();
      if (me) {
        const ui = await loadSessionState(); // applies the saved locale
        const last = ui.global.lastWorld;
        if (last) {
          // A transient /api/worlds failure here degrades to entry, not a hard error.
          try {
            const worlds = await listWorlds();
            if (worlds.some((w) => w.id === last)) {
              enterWorld(last); // reload returns you to your last world
              return;
            }
            setLastWorld(null); // stale (deleted / revoked) — clear it
          } catch {
            // fall through to entry
          }
        }
      }
      navigate({ name: me ? "worlds" : "login" }); // pre-world; <Entry> picks the step
    } catch {
      // A transient backend failure must not wedge the SPA on "Loading…".
      navigate({ name: "login" });
    } finally {
      booted = true;
    }
  }
  boot();

  // Best-effort persist of a still-pending ui_state change when the tab is hidden
  // or unloaded (the debounce's trailing write would otherwise not fire).
  if (typeof window !== "undefined") {
    window.addEventListener("pagehide", flushOnUnload);
    window.addEventListener("visibilitychange", () => {
      if (document.visibilityState === "hidden") flushOnUnload();
    });
  }

  // Entry authenticated the user; fetch identity + apply saved session state
  // (locale) before entry advances to world-select — the pre-split boot order.
  async function onAuthenticated() {
    try {
      me = await getMe();
      await loadSessionState();
    } catch {
      me = null;
    }
  }

  function enterWorld(worldId: string) {
    if (!me) return;
    const wsUrl =
      (location.protocol === "https:" ? "wss:" : "ws:") +
      "//" + location.host + "/ws?world=" + worldId;
    const s = new WorldSession({ selfId: me.id, connect: webSocketConnect(wsUrl), coreUiModule: coreUi, featureModules: [sceneTools] });
    session = s;
    void s.enter(worldId);
    setLastWorld(worldId);
    navigate({ name: "world", id: worldId });
  }

  function leaveWorld() {
    session?.leave();
    session = null;
    setLastWorld(null);
    navigate({ name: "worlds" });
  }

  const route = $derived(currentRoute());
</script>

{#if !booted}
  <p class="connecting">Loading…</p>
{:else if route.name === "world" && session?.role && session?.world}
  <Table {session} {leaveWorld} />
{:else if route.name === "world"}
  <p class="connecting">Connecting…</p>
{:else}
  <Entry {onAuthenticated} onEnterWorld={enterWorld} />
{/if}
