<script lang="ts">
  import type { WireDocument } from "@shadowcat/core";

  let { message, showChannel }: { message: WireDocument; showChannel: boolean } = $props();
  const text = $derived(((message.engine as { content?: { kind: string; text?: string }[] } | undefined)?.content ?? [])
    .map((s) => (s.kind === "text" ? s.text : ""))
    .join(""));
</script>

<div data-testid="card" data-show-channel={showChannel}>{text}</div>
