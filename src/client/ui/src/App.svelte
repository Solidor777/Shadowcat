<script lang="ts">
  import { webSocketConnect } from "@shadowcat/core";
  import { getConfig, getMe, type Me } from "./lib/api";
  import { currentRoute, navigate } from "./lib/route.svelte";
  import { coreUi } from "./modules/core-ui/index";
  import { WorldSession } from "./lib/worldSession.svelte";
  import Setup from "./lib/views/Setup.svelte";
  import Login from "./lib/views/Login.svelte";
  import WorldSelect from "./lib/views/WorldSelect.svelte";
  import Table from "./lib/Table.svelte";

  let me = $state<Me | null>(null);
  let booted = $state(false);
  let session = $state<WorldSession | null>(null);

  async function boot() {
    const cfg = await getConfig();
    if (!cfg.initialized) {
      navigate({ name: "setup" });
      booted = true;
      return;
    }
    me = await getMe();
    navigate({ name: me ? "worlds" : "login" });
    booted = true;
  }
  boot();

  async function afterAuth() {
    me = await getMe();
    navigate({ name: "worlds" });
  }

  function enterWorld(worldId: string) {
    if (!me) return;
    const wsUrl =
      (location.protocol === "https:" ? "wss:" : "ws:") +
      "//" + location.host + "/ws?world=" + worldId;
    const s = new WorldSession({ selfId: me.id, connect: webSocketConnect(wsUrl), coreUiModule: coreUi });
    session = s;
    void s.enter(worldId);
    navigate({ name: "world", id: worldId });
  }

  const route = $derived(currentRoute());
</script>

{#if !booted}
  <p class="connecting">Loading…</p>
{:else if route.name === "setup"}
  <Setup onDone={() => navigate({ name: "login" })} />
{:else if route.name === "world" && session?.role && session?.world}
  <Table {session} />
{:else if route.name === "world"}
  <p class="connecting">Connecting…</p>
{:else if route.name === "worlds"}
  <WorldSelect onEnter={enterWorld} />
{:else}
  <Login onAuthed={afterAuth} />
{/if}
