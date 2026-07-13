<script lang="ts">
  import { setAppContext, Surface, PanelsBridge } from "@shadowcat/ui-kit";
  import { t } from "@shadowcat/ui-kit";
  import { consoleLogger } from "@shadowcat/core";
  import { logout } from "./api";
  import { navigate } from "./route.svelte";
  import { getActiveTab, setActiveTab, getPanelLayout, setPanelLayout } from "./sessionState.svelte";
  import type { WorldSession } from "./worldSession.svelte";

  // TODO: bind the real panel host once it mounts; until then calls warn-once and no-op.
  const panels = new PanelsBridge(consoleLogger());

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
    pathfind: (s, st, wp, fr) => session.pathfind(s, st, wp, fr),
    moveRequest: (s, tid, p) => session.moveRequest(s, tid, p),
    onPing: (cb) => session.onPing(cb),
    chat: {
      send: (o) => session.sendChatMessage(o),
      edit: (id, c) => session.editChatMessage(id, c),
      delete: (id) => session.deleteChatMessage(id),
    },
    uiState: {
      getActiveTab: () => getActiveTab(session.world!),
      setActiveTab: (id) => setActiveTab(session.world!, id),
      getPanelLayout: () => getPanelLayout(session.world!),
      setPanelLayout: (blob) => setPanelLayout(session.world!, blob),
    },
    panels,
    leaveWorld,
    logout: async () => {
      await logout();
      navigate({ name: "login" });
    },
  });
</script>

<Surface contract="shadowcat.surface:root" />
