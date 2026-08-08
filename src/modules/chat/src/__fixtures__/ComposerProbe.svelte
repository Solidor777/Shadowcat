<script lang="ts">
  import type { WireAudience } from "@shadowcat/core";

  let {
    channel,
    audience,
    placeholderName,
  }: {
    /** Forwarded to `data-channel` for assertions. */
    channel: string;
    /** Forwarded to `data-audience` (its `kind` only) for assertions. */
    audience: WireAudience;
    /** Forwarded to `data-placeholder` for assertions. */
    placeholderName: string;
  } = $props();
</script>

<div data-testid="composer" data-channel={channel} data-audience={audience.kind} data-placeholder={placeholderName}></div>
