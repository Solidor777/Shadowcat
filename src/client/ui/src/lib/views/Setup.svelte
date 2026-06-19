<script lang="ts">
  import { setup } from "../api";
  import { t } from "../i18n.svelte";

  let { onDone }: { onDone: () => void } = $props();
  let username = $state("");
  let password = $state("");
  let token = $state("");
  let error = $state("");
  let busy = $state(false);

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    busy = true;
    error = "";
    const { ok, status } = await setup(username, password, token || undefined);
    busy = false;
    if (ok) onDone();
    else error = status === 403 ? t("setup.errorToken") : t("setup.errorGeneric", { status });
  }
</script>

<main class="entry">
  <h1>{t("setup.title")}</h1>
  <form onsubmit={submit}>
    <label>{t("common.username")} <input bind:value={username} autocomplete="username" /></label>
    <label>{t("common.password")}
      <input type="password" bind:value={password} autocomplete="new-password" />
    </label>
    <label>{t("setup.token")} <input bind:value={token} /></label>
    {#if error}<p role="alert">{error}</p>{/if}
    <button type="submit" disabled={busy}>{t("setup.submit")}</button>
  </form>
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
  form {
    display: grid;
    gap: var(--space-3);
  }
  label {
    display: grid;
    gap: var(--space-1);
    color: var(--text-muted);
    font-size: 0.875rem;
  }
</style>
