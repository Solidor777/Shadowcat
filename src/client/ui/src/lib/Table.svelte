<script lang="ts">
  import { setAppContext } from "./appContext";
  import Layout from "./Layout.svelte";
  import type { WorldSession } from "./worldSession.svelte";

  let { session }: { session: WorldSession } = $props();
  // App renders <Table> only once role+world are set (Welcome received), so these
  // are non-null at init. setContext must run during init, not in markup.
  setAppContext({
    contributions: session.contributions,
    store: session.store,
    world: session.world!,
    role: session.role!,
  });
</script>

<Layout />
