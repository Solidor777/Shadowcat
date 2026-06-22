<script lang="ts">
  import { ContributionRegistry, DocumentStore, AssetResolver } from "@shadowcat/core";
  import { setAppContext } from "../appContext";
  import { t } from "../i18n.svelte";
  import Surface from "../Surface.svelte";

  let { registry, contract }: { registry: ContributionRegistry; contract: string } =
    $props();
  // The registry is a fixed instance per render; capturing it once is intended.
  // store/world/role/t/assets are unused by <Surface> but required by the AppContext shape.
  // svelte-ignore state_referenced_locally
  setAppContext({ contributions: registry, store: new DocumentStore(), world: "test", role: "gm", t, assets: new AssetResolver(), onAssetChanged: () => () => {}, subscribeScene: () => ({ unsubscribe() {} }), leaveWorld: () => {} });
</script>

<Surface {contract} />
