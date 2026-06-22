<script lang="ts">
  import { AssetResolver } from "@shadowcat/core";
  import { setAppContext } from "../../../../lib/appContext";
  import { t } from "../../../../lib/i18n.svelte";
  import Assets from "../Assets.svelte";

  let { onAssetChanged = () => () => {} }: {
    onAssetChanged?: (cb: (m: { uuid: string; op: "replaced" | "deleted" }) => void) => () => void;
  } = $props();
  // svelte-ignore state_referenced_locally
  setAppContext({
    contributions: undefined as never,
    store: undefined as never,
    world: "w1",
    role: "gm",
    t,
    assets: new AssetResolver(),
    onAssetChanged,
    leaveWorld: () => {},
  });
</script>

<Assets />
