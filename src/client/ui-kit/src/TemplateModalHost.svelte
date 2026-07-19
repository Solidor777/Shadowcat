<script lang="ts">
  // Mounts the conflict modal from the templates controller's reactive `pending` session. The
  // shell renders exactly one of these; the seam exposes `_pending`/`_cancel` for it.
  import { getAppContext } from "./appContext";
  import MergeConflictModal from "./MergeConflictModal.svelte";
  import type { TemplatesController } from "./templatesController.svelte";

  const ctx = getAppContext();
  // The shell provides the concrete controller behind the seam.
  let { controller }: { controller: TemplatesController } = $props();
  void ctx;
</script>

{#if controller.pending}
  <MergeConflictModal
    groups={controller.pending.groups}
    onApply={(m) => controller.pending?.resolve(m)}
    onCancel={() => controller.cancel()}
  />
{/if}
