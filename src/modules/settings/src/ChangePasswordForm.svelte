<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import { changeOwnPassword, ChangePasswordError } from "@shadowcat/core";

  // Self-service password change, shown to every signed-in account. The server re-verifies the
  // current password and applies the password policy; the confirmation field only catches a
  // typo before the request and is never a second validation authority. A successful change
  // revokes every session for the account, this one included, so the form ends the local
  // session through the context's `logout` (which returns to the login view).
  const { t, logout } = getAppContext();

  let current = $state("");
  let next = $state("");
  let confirm = $state("");
  let busy = $state(false);
  let error = $state<string | null>(null);

  /**
   * Submits the change. A mismatched confirmation blocks the request. A 401 shows the
   * fixed "current password incorrect" text; a 429 shows the fixed "too many attempts" text; any other refusal (a 422 policy failure
   * included) shows the server's message. On success the typed values are dropped
   * from component state and the local session is ended.
   * @param e The form's submit event; only `preventDefault` is used.
   * @returns Resolves once the outcome is shown or the local session is ended; never rejects.
   * @example
   * ```
   * // private function; not part of the public API — wired to the form's onsubmit
   * declare const submitEvent: SubmitEvent;
   * await submit(submitEvent);
   * ```
   */
  async function submit(e: SubmitEvent): Promise<void> {
    e.preventDefault();
    error = null;
    if (next !== confirm) {
      error = t("settings.account.mismatch");
      return;
    }
    busy = true;
    try {
      await changeOwnPassword(current, next);
    } catch (err) {
      if (err instanceof ChangePasswordError && err.status === 401) {
        error = t("settings.account.currentIncorrect");
      } else if (err instanceof ChangePasswordError && err.status === 429) {
        error = t("settings.account.tooManyAttempts");
      } else {
        error = t("settings.account.failed", {
          message: err instanceof Error ? err.message : String(err),
        });
      }
      busy = false;
      return;
    }
    current = "";
    next = "";
    confirm = "";
    busy = false;
    await logout();
  }
</script>

<section class="change-password">
  <h3>{t("settings.account.title")}</h3>
  <form onsubmit={submit}>
    <label>
      {t("settings.account.current")}
      <input type="password" bind:value={current} autocomplete="current-password" required />
    </label>
    <label>
      {t("settings.account.new")}
      <input type="password" bind:value={next} autocomplete="new-password" required />
    </label>
    <label>
      {t("settings.account.confirm")}
      <input type="password" bind:value={confirm} autocomplete="new-password" required />
    </label>
    <button type="submit" disabled={busy}>{t("settings.account.submit")}</button>
  </form>
  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}
</section>

<style lang="scss">
  .change-password {
    display: grid;
    gap: var(--space-2);
  }
  form {
    display: grid;
    gap: var(--space-2);
  }
  .error {
    color: var(--danger);
  }
  input {
    min-height: 32px;
  }
  button {
    min-height: 32px;
  }
  @media (pointer: coarse) {
    input,
    button {
      min-height: var(--input-height-coarse);
    }
  }
</style>
