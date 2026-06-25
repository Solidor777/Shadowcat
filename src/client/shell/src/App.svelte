<script lang="ts">
  import { webSocketConnect } from "@shadowcat/core";
  import { Entry } from "@shadowcat/module-entry";
  import { getMe, listWorlds, type Me } from "./lib/api";
  import { loadSessionState, setLastWorld, flushOnUnload } from "./lib/sessionState.svelte";
  import { currentRoute, navigate } from "./lib/route.svelte";
  import { coreUi } from "@shadowcat/module-core-ui";
  import { topBar } from "@shadowcat/module-topbar";
  import { statusBar } from "@shadowcat/module-statusbar";
  import { stage } from "@shadowcat/module-stage";
  import { settings } from "@shadowcat/module-settings";
  import { assets } from "@shadowcat/module-assets";
  import { actors } from "@shadowcat/module-actors";
  import { factions } from "@shadowcat/module-factions";
  import { conditions } from "@shadowcat/module-conditions";
  import { gameSettings } from "@shadowcat/module-game-settings";
  import { sceneTools } from "@shadowcat/module-scene-tools";
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
      // Seeds the URL hash only; <Entry> derives the actual pre-world step (setup/
      // login/world-select) internally — every pre-world route renders <Entry>.
      navigate({ name: me ? "worlds" : "login" });
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
  // Returns whether identity is in hand: a failed fetch sends entry back to login
  // (the old afterAuth `me ? "worlds" : "login"` recovery branch).
  async function onAuthenticated(): Promise<boolean> {
    try {
      me = await getMe();
      await loadSessionState();
    } catch {
      me = null;
    }
    return me !== null;
  }

  function enterWorld(worldId: string) {
    if (!me) return;
    const wsUrl =
      (location.protocol === "https:" ? "wss:" : "ws:") +
      "//" + location.host + "/ws?world=" + worldId;
    const s = new WorldSession({ selfId: me.id, connect: webSocketConnect(wsUrl), modules: [coreUi, topBar, statusBar, stage, settings, gameSettings, assets, actors, factions, conditions, sceneTools] });
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
