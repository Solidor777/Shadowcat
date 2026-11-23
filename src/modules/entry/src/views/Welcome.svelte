<script lang="ts">
  import { t } from "@shadowcat/ui-kit";

  let {
    onContinue,
  }: {
    /** Invoked when the user leaves the welcome screen; `Entry` advances to `login`. */
    onContinue: () => void;
  } = $props();

  /** The hosting guide in the project repository; the screen shows before any world exists, so
   * the guide is read online rather than served by this server. */
  const HOSTING_GUIDE_URL = "https://github.com/Solidor777/Shadowcat/blob/main/docs/site/guides/hosting.md";
</script>

<main class="entry" data-testid="entry-welcome">
  <h1>{t("entry.welcome.title")}</h1>
  <p>{t("entry.welcome.body")}</p>
  <p><a href={HOSTING_GUIDE_URL} target="_blank" rel="noopener noreferrer">{t("entry.welcome.guide")}</a></p>
  <button type="button" onclick={onContinue}>{t("entry.welcome.continue")}</button>
</main>

<style lang="scss">
  .entry {
    max-width: 22rem;
    margin: 4rem auto;
    display: grid;
    gap: var(--space-4);
    padding: var(--space-6);
    background: var(--surface-raised);
    border: 1px solid var(--border);
    border-radius: var(--radius-2);
  }
  p {
    margin: 0;
  }
  button {
    min-height: var(--input-height-coarse, 44px);
  }
</style>
