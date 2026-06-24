<script lang="ts">
  import { AssetResolver } from "@shadowcat/core";
  import { setAppContext } from "@shadowcat/ui-kit";
  import { SceneInteractionBridge, ActorSelection, TokenSelection } from "@shadowcat/ui-kit";
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
    selfId: "u1",
    canEdit: () => true,
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
    onPing: () => () => {},
    leaveWorld: () => {},
    logout: async () => {},
  });
</script>

<Assets />
