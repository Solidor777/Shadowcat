<script lang="ts">
  // Mounts the conflict modal from the templates controller's reactive `pending` session. The
  // shell passes the concrete `TemplatesController` instance directly as the `controller` prop.
  import MergeConflictModal from "./MergeConflictModal.svelte";
  import type { TemplatesController } from "./templatesController.svelte";

  let { controller }: {
    /** The shell's `TemplatesController` instance whose `pending` session drives the modal. */
    controller: TemplatesController;
  } = $props();
</script>

{#if controller.pending}
  <MergeConflictModal
    groups={controller.pending.groups}
    onApply={(m) => controller.pending?.resolve(m)}
    onCancel={() => controller.cancel()}
  />
{/if}
