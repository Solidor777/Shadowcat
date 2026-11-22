<script lang="ts">
  // Minimal chat-card extension: renders the opaque `system` payload's `tag` field so a test can
  // assert which extension mounted and that it received `system` and the parsed message body.
  let { system, message, name }: {
    /** The message document's opaque `system` band, as `MessageCard` passes it. */
    system: { /** Label the probe renders. */ tag?: string };
    /** The message's parsed `engine` body, as `MessageCard` passes it. */
    message: { /** The channel the message was posted to. */ channel: string };
    /** Contributed through `Contribution.props`; identifies the probe instance. */
    name: string;
  } = $props();
</script>

<span data-testid="ext-probe">{name}:{system.tag}:{message.channel}</span>
