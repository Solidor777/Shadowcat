<script lang="ts">
  import { login } from "../api";

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
  <h1>shadowcat</h1>
  <form onsubmit={submit}>
    <label>Username <input bind:value={username} autocomplete="username" /></label>
    <label>Password
      <input type="password" bind:value={password} autocomplete="current-password" />
    </label>
    {#if error}<p role="alert">Invalid username or password.</p>{/if}
    <button type="submit" disabled={busy}>Log in</button>
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
