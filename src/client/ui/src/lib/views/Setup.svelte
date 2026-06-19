<script lang="ts">
  import { setup } from "../api";

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
    else error = status === 403 ? "Invalid setup token." : `Setup failed (${status}).`;
  }
</script>

<main class="entry">
  <h1>Create the admin account</h1>
  <form onsubmit={submit}>
    <label>Username <input bind:value={username} autocomplete="username" /></label>
    <label>Password
      <input type="password" bind:value={password} autocomplete="new-password" />
    </label>
    <label>Setup token (if required) <input bind:value={token} /></label>
    {#if error}<p role="alert">{error}</p>{/if}
    <button type="submit" disabled={busy}>Create admin</button>
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
