<script lang="ts">
  import { setAppContext } from "./appContext";
  import { t } from "./i18n.svelte";
  import Layout from "./Layout.svelte";
  import type { WorldSession } from "./worldSession.svelte";

  let { session, leaveWorld }: { session: WorldSession; leaveWorld: () => void } =
    $props();
  // App renders <Table> only once role+world are set (Welcome received), so these
  // are non-null at init. setContext must run during init, not in markup; the
  // session/leaveWorld are fixed per Table, so capturing them once is intended.
  // svelte-ignore state_referenced_locally
  setAppContext({
    contributions: session.contributions,
    store: session.store,
    world: session.world!,
    role: session.role!,
    t,
    assets: session.assets,
    onAssetChanged: (cb) => session.onAssetChanged(cb),
    subscribeScene: (c, cb) => session.subscribeScene(c, cb),
    leaveWorld,
  });
</script>

<Layout />
