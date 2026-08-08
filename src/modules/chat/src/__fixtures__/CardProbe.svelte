<script lang="ts">
  import type { WireDocument } from "@shadowcat/core";

  let {
    message,
    showChannel,
  }: {
    /** The probed message document. */
    message: WireDocument;
    /** Forwarded to `data-show-channel` for assertions; this probe does no channel-chip rendering itself. */
    showChannel: boolean;
  } = $props();
  const text = $derived(((message.engine as {
    /** The raw, unparsed segment list — this probe reads `text` segments only, skipping schema validation. */
    content?: {
      /** Segment discriminant; only `"text"` is read below. */
      kind: string;
      /** Present on a `"text"` segment. */
      text?: string;
    }[];
  } | undefined)?.content ?? [])
    .map((s) => (s.kind === "text" ? s.text : ""))
    .join(""));
</script>

<div data-testid="card" data-show-channel={showChannel}>{text}</div>
