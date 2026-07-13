<script lang="ts">
  import type { WireAudience } from "@shadowcat/core";

  let { channel, audience, placeholderName }: { channel: string; audience: WireAudience; placeholderName: string } = $props();
</script>

<div data-testid="composer" data-channel={channel} data-audience={audience.kind} data-placeholder={placeholderName}></div>
