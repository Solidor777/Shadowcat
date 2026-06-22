<script lang="ts">
  import { AssetResolver } from "@shadowcat/core";
  import { setAppContext } from "../../../../lib/appContext";
  import { SceneInteractionBridge } from "../../../../lib/sceneInteraction";
  import { t } from "../../../../lib/i18n.svelte";
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
    members: new Map(),
    t,
    assets: new AssetResolver(),
    onAssetChanged,
    subscribeScene: () => ({ unsubscribe() {} }),
    dispatchIntent: () => {},
    scene: new SceneInteractionBridge(),
    sendPing: () => {},
    onPing: () => () => {},
    leaveWorld: () => {},
  });
</script>

<Assets />
