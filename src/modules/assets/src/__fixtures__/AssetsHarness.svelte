<script lang="ts">
  import type { AssetChangedNotice } from "@shadowcat/core";
  import { AssetResolver, silentLogger, EMPTY_FOOTPRINTS } from "@shadowcat/core";
  import { setAppContext } from "@shadowcat/ui-kit";
  import { SceneInteractionBridge, ActorSelection, TokenSelection, PanelsBridge, SceneSelection, SpeakAs, SpeakAsToken } from "@shadowcat/ui-kit";
  import { t } from "@shadowcat/ui-kit";
  import Assets from "../Assets.svelte";

  let {
    onAssetChanged = () => () => {},
    assets = new AssetResolver(),
  }: {
    /** Fixture stand-in for `AppContext.onAssetChanged`; defaults to a subscriber that never
     * fires, since no real `AssetChanged` broadcast reaches this harness. */
    onAssetChanged?: (cb: (m: AssetChangedNotice) => void) => () => void;
    /** Fixture stand-in for `AppContext.assets`; a caller-supplied instance lets a test pre-seed
     * `revs`/`deleted` state before render, to prove `Assets.svelte`'s `reload` self-heals it via
     * `reconcile`. Defaults to a fresh `AssetResolver`. */
    assets?: AssetResolver;
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
    notify: () => {},
    members: new Map(),
    t,
    assets,
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
    chat: {
      send: () => Promise.resolve(),
      edit: () => Promise.resolve(),
      delete: () => Promise.resolve(),
      recalc: () => Promise.resolve(),
    },
    reconcileInstalledModules: () => Promise.resolve(),
    leaveWorld: () => {},
    logout: async () => {},
    uiState: { getPanelLayout: () => null, setPanelLayout: () => {}, getChatRead: () => null, setChatRead: () => {} },
    panels: new PanelsBridge(silentLogger),
    viewedSceneId: null,
    setGmViewedScene: () => {},
    searchDocuments: () => Promise.reject(new Error("not connected")),
    sceneSelection: new SceneSelection(),
    speakAsToken: new SpeakAsToken(),
    speakAs: new SpeakAs(),
    footprints: EMPTY_FOOTPRINTS,
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
