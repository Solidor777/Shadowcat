<script lang="ts">
  // Stands in for @shadowcat/module-entry's <Entry> in App.test so the shell's boot
  // branching is testable without exercising entry's own getConfig/getMe fetches
  // (covered by the entry package's own tests).
  let {}: { onAuthenticated: () => void; onEnterWorld: (id: string) => void } = $props();
</script>

<div data-testid="entry-stub">entry</div>
