<script lang="ts">
  // Renders a `WorldSession.canCreate`/`canEdit` read reactively via `$derived`, so a test can
  // observe whether a capability-only Welcome (no other reactive field changing) refreshes it —
  // a `WorldSession` method call from a plain `.test.ts` file re-reads current state on every
  // call regardless of reactivity, so only a REAL component read (compiled by the Svelte
  // preprocessor, which `$effect`/`$derived` require) can distinguish a stale mirror from a
  // reactive one.
  import type { WorldSession } from "../worldSession.svelte";
  import type { WireDocument } from "@shadowcat/core";

  let {
    session,
    docType,
    doc,
    path,
  }: {
    /** The session whose `canCreate`/`canEdit` this probe reads. */
    session: WorldSession;
    /** Passed to `canCreate` when set. */
    docType?: string;
    /** Passed to `canEdit` alongside `path` when both are set. */
    doc?: WireDocument;
    /** Passed to `canEdit` alongside `doc` when both are set. */
    path?: string;
  } = $props();

  const canCreateResult = $derived(docType !== undefined ? session.canCreate(docType) : null);
  const canEditResult = $derived(doc !== undefined && path !== undefined ? session.canEdit(doc, path) : null);
</script>

<span data-testid="probe-can-create">{String(canCreateResult)}</span>
<span data-testid="probe-can-edit">{String(canEditResult)}</span>
