<script lang="ts">
  import { ContributionRegistry, DocumentStore, AssetResolver } from "@shadowcat/core";
  import { setAppContext } from "../appContext";
  import { SceneInteractionBridge } from "../sceneInteraction";
  import { t } from "../i18n.svelte";
  import Surface from "../Surface.svelte";

  let { registry, contract }: { registry: ContributionRegistry; contract: string } =
    $props();
  // The registry is a fixed instance per render; capturing it once is intended.
  // store/world/role/t/assets are unused by <Surface> but required by the AppContext shape.
  // svelte-ignore state_referenced_locally
  setAppContext({ contributions: registry, store: new DocumentStore(), documents: new DocumentStore(), world: "test", role: "gm", members: new Map(), t, assets: new AssetResolver(), onAssetChanged: () => () => {}, subscribeScene: () => ({ unsubscribe() {} }), dispatchIntent: () => {}, scene: new SceneInteractionBridge(), sendPing: () => {}, onPing: () => () => {}, leaveWorld: () => {}, logout: async () => {} });
</script>

<Surface {contract} />
