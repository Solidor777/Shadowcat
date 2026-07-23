<script lang="ts">
  import { AssetResolver, silentLogger } from "@shadowcat/core";
  import { setAppContext } from "@shadowcat/ui-kit";
  import { SceneInteractionBridge, ActorSelection, TokenSelection, PanelsBridge, SceneSelection } from "@shadowcat/ui-kit";
  import { t } from "@shadowcat/ui-kit";
  import Assets from "../Assets.svelte";

  let { onAssetChanged = () => () => {} }: {
    onAssetChanged?: (cb: (m: { uuid: string; op: "replaced" | "deleted" }) => void) => () => void;
  } = $props();
  // svelte-ignore state_referenced_locally
  setAppContext({
    contributions: undefined as never,
    store: undefined as never,
    documents: undefined as never,
    world: "w1",
    role: "gm",
    serverRole: "user",
    selfId: "u1",
    canEdit: () => true,
    openDocument: () => {},
    members: new Map(),
    t,
    assets: new AssetResolver(),
    onAssetChanged,
    subscribeScene: () => ({ unsubscribe() {} }),
    dispatchIntent: () => {},
    scene: new SceneInteractionBridge(),
    actorSelection: new ActorSelection(),
    tokenSelection: new TokenSelection(),
    sendPing: () => {},
    pathfind: () => Promise.reject(new Error("not connected")),
    moveRequest: () => Promise.reject(new Error("not connected")),
    onPing: () => () => {},
    onMoveOutcome: () => () => {},
    chat: { send: () => Promise.resolve(), edit: () => Promise.resolve(), delete: () => Promise.resolve() },
    leaveWorld: () => {},
    logout: async () => {},
    uiState: { getPanelLayout: () => null, setPanelLayout: () => {}, getChatRead: () => null, setChatRead: () => {} },
    panels: new PanelsBridge(silentLogger),
    viewedSceneId: null,
    setGmViewedScene: () => {},
    searchDocuments: () => Promise.reject(new Error("not connected")),
    sceneSelection: new SceneSelection(),
    templates: {
      stampInstance: (s) => s,
      pull: () => {},
      push: () => {},
      revert: () => {},
      findInstances: () => [],
      syncState: () => "none",
      canPull: () => false,
      canPush: () => false,
    },
  });
</script>

<Assets />
