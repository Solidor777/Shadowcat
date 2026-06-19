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

<style>
  .entry { max-width: 22rem; margin: 4rem auto; display: grid; gap: 1rem; }
  form { display: grid; gap: 0.75rem; }
  label { display: grid; gap: 0.25rem; }
</style>
