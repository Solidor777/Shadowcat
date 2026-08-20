<script lang="ts">
  // Reads the AppContext set by an ancestor (Table) and hands it to the test via a
  // callback — the only way to reach a component's `setContext` value from outside
  // the component tree, since Svelte context is readable only from a descendant.
  import { getAppContext, type AppContext } from "@shadowcat/ui-kit";

  let { onContext }: {
    /** Called once at init with the captured AppContext. */
    onContext: (ctx: AppContext) => void;
  } = $props();

  // Fixed per mount, like Table's own `setAppContext` capture — the intent is a
  // one-shot read at init, not a reactive re-run on prop change.
  // svelte-ignore state_referenced_locally
  onContext(getAppContext());
</script>
