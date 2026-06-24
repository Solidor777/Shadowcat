<script lang="ts">
  import { login } from "../api";
  import { t } from "@shadowcat/ui-kit";

  let { onAuthed }: { onAuthed: () => void } = $props();
  let username = $state("");
  let password = $state("");
  let error = $state(false);
  let busy = $state(false);

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    busy = true;
    error = false;
    const ok = await login(username, password);
    busy = false;
    if (ok) onAuthed();
    else error = true;
  }
</script>

<main class="entry">
  <h1>{t("app.name")}</h1>
  <form onsubmit={submit}>
    <label>{t("common.username")} <input bind:value={username} autocomplete="username" /></label>
    <label>{t("common.password")}
      <input type="password" bind:value={password} autocomplete="current-password" />
    </label>
    {#if error}<p role="alert">{t("login.error")}</p>{/if}
    <button type="submit" disabled={busy}>{t("login.submit")}</button>
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
