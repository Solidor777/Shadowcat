<script lang="ts">
  import { ContributionRegistry, DocumentStore, AssetResolver, silentLogger } from "@shadowcat/core";
  import { setAppContext } from "../appContext";
  import { SceneInteractionBridge } from "../sceneInteraction";
  import { ActorSelection } from "../actorSelection.svelte";
  import { TokenSelection } from "../tokenSelection.svelte";
  import { PanelsBridge } from "../panelsBridge.svelte";
  import { SceneSelection } from "../sceneSelection.svelte";
  import { t } from "../i18n.svelte";
  import Surface from "../Surface.svelte";

  let { registry, contract }: { registry: ContributionRegistry; contract: string } =
    $props();
  // The registry is a fixed instance per render; capturing it once is intended.
  // store/world/role/t/assets are unused by <Surface> but required by the AppContext shape.
  // svelte-ignore state_referenced_locally
  setAppContext({ contributions: registry, store: new DocumentStore(), documents: new DocumentStore(), world: "test", role: "gm", selfId: "u1", canEdit: () => true, openDocument: () => {}, members: new Map(), t, assets: new AssetResolver(), onAssetChanged: () => () => {}, subscribeScene: () => ({ unsubscribe() {} }), dispatchIntent: () => {}, scene: new SceneInteractionBridge(), actorSelection: new ActorSelection(), tokenSelection: new TokenSelection(), sendPing: () => {}, pathfind: () => Promise.reject(new Error("not connected")), moveRequest: () => Promise.reject(new Error("not connected")), onPing: () => () => {}, chat: { send: () => Promise.resolve(), edit: () => Promise.resolve(), delete: () => Promise.resolve() }, leaveWorld: () => {}, logout: async () => {}, uiState: { getPanelLayout: () => null, setPanelLayout: () => {}, getChatRead: () => null, setChatRead: () => {} }, panels: new PanelsBridge(silentLogger), viewedSceneId: null, setGmViewedScene: () => {}, searchDocuments: () => Promise.reject(new Error("not connected")), sceneSelection: new SceneSelection(), templates: { stampInstance: (s) => s, pull: () => {}, push: () => {}, revert: () => {}, findInstances: () => [], syncState: () => "none", canPull: () => false, canPush: () => false } });
</script>

<Surface {contract} />
