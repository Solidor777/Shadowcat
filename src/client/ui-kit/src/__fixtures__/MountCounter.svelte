<script lang="ts" module>
  // Module-level counter: increments once per component instantiation.
  // A collapse->expand round trip that remounts the panel would bump this;
  // staying mounted (hidden non-destructively) leaves it unchanged.
  export let mountCount = 0;
  export function resetMountCount(): void {
    mountCount = 0;
  }
</script>

<script lang="ts">
  mountCount += 1;
</script>

<span data-testid="mount-counter">mounted</span>
