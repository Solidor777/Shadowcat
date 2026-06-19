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

<style>
  .entry { max-width: 22rem; margin: 4rem auto; display: grid; gap: 1rem; }
  form { display: grid; gap: 0.75rem; }
  label { display: grid; gap: 0.25rem; }
</style>
