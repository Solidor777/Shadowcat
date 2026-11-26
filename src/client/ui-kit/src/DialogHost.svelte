<script lang="ts">
  // Renders every dialog opened through `confirmDialog` / `promptDialog`. Mounted once by the
  // shell beside `NotificationHost`, outside any surface, so no docked panel can clip a dialog.
  import ConfirmDialog from "./ConfirmDialog.svelte";
  import PromptDialog from "./PromptDialog.svelte";
  import { dialogQueue } from "./dialogQueue.svelte";
</script>

{#each dialogQueue.items as request (request.id)}
  {#if request.kind === "confirm"}
    <ConfirmDialog
      title={request.title}
      message={request.message}
      confirmLabel={request.confirmLabel}
      cancelLabel={request.cancelLabel}
      danger={request.danger}
      onConfirm={() => request.settle(true)}
      onCancel={() => request.settle(false)}
    />
  {:else}
    <PromptDialog
      title={request.title}
      message={request.message}
      confirmLabel={request.confirmLabel}
      cancelLabel={request.cancelLabel}
      danger={request.danger}
      initialValue={request.initialValue}
      placeholder={request.placeholder}
      validate={request.validate}
      onConfirm={(value) => request.settle(value)}
      onCancel={() => request.settle(null)}
    />
  {/if}
{/each}
