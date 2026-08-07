<script lang="ts">
  // Minimal contributable component: exercises <Surface>/registry rendering without
  // any real module dependency.
  let { label }: {
    /** Text rendered verbatim, so a test can assert which contribution mounted. */
    label: string;
  } = $props();
</script>

<span data-testid="probe">{label}</span>
