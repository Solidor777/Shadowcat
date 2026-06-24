<script lang="ts">
  import { setAppContext, Surface } from "@shadowcat/ui-kit";
  import { t } from "@shadowcat/ui-kit";
  import { logout } from "./api";
  import { navigate } from "./route.svelte";
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
    documents: session.documents,
    world: session.world!,
    role: session.role!,
    selfId: session.selfId,
    canEdit: (doc, path) => session.canEdit(doc, path),
    members: session.members,
    t,
    assets: session.assets,
    onAssetChanged: (cb) => session.onAssetChanged(cb),
    subscribeScene: (c, cb, opts) => session.subscribeScene(c, cb, opts),
    dispatchIntent: (ops) => session.dispatchIntent(ops),
    scene: session.sceneInteraction,
    actorSelection: session.actorSelection,
    tokenSelection: session.tokenSelection,
    sendPing: (x, y) => session.sendPing(x, y),
    onPing: (cb) => session.onPing(cb),
    leaveWorld,
    logout: async () => {
      await logout();
      navigate({ name: "login" });
    },
  });
</script>

<Surface contract="shadowcat.surface:root" />
