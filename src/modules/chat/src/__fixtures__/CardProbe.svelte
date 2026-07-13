<script lang="ts">
  import type { WireDocument } from "@shadowcat/core";

  let { message, showChannel }: { message: WireDocument; showChannel: boolean } = $props();
  const text = $derived(((message.system as { content?: { kind: string; text?: string }[] }).content ?? [])
    .map((s) => (s.kind === "text" ? s.text : ""))
    .join(""));
</script>

<div data-testid="card" data-show-channel={showChannel}>{text}</div>
