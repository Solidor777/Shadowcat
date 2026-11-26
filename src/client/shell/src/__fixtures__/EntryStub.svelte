<script lang="ts">
  // Stands in for @shadowcat/module-entry's <Entry> in App.test so the shell's boot
  // branching is testable without exercising entry's own getConfig/getMe fetches
  // (covered by the entry package's own tests). Records `onEnterWorld` in `captured`.
  import { captured } from "./entryCapture";

  let {
    onEnterWorld,
  }: {
    /** Unused by the stub; present only to match `<Entry>`'s real prop shape. */
    onAuthenticated: () => boolean | Promise<boolean>;
    /** Recorded in `captured` so a test can enter a world as the real entry screen would. */
    onEnterWorld: (id: string) => void;
  } = $props();

  captured.onEnterWorld = (id) => onEnterWorld(id);
</script>

<div data-testid="entry-stub">entry</div>
