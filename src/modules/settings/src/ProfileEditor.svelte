<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import { avatarUrl, updateProfile, uploadAvatar, deleteAvatar } from "@shadowcat/core";

  // Self-service display profile: colour accent, pronouns and avatar, shown to every signed-in
  // account. The server validates and stores each value; the pronouns length cap here only
  // saves a round trip. Saved values are written into `ctx.memberProfiles` so this client's
  // roster and chat show them at once; other clients read them on their next roster fetch.
  const ctx = getAppContext();
  const { t } = ctx;

  /** Mirrors the server's pronouns length cap (`validate_pronouns`). */
  const MAX_PRONOUNS_CHARS = 32;
  /** Colour shown on the picker while no colour is set; never sent unless the user picks one. */
  const PICKER_PLACEHOLDER = "#808080";

  const stored = $derived(
    ctx.memberProfiles.get(ctx.selfId) ?? { colour: null, pronouns: null, avatar: null },
  );
  // `undefined` means "not edited": the field follows the stored profile, which can arrive
  // after this form mounts (the roster loads on connect).
  let colourDraft = $state<string | null | undefined>(undefined);
  let pronounsDraft = $state<string | undefined>(undefined);
  const colour = $derived(colourDraft === undefined ? stored.colour : colourDraft);
  const pronouns = $derived(pronounsDraft === undefined ? (stored.pronouns ?? "") : pronounsDraft);

  let busy = $state(false);
  let error = $state<string | null>(null);

  /**
   * Describes a failure for display.
   * @param err The thrown value.
   * @returns The localized failure text carrying the server's message.
   * @example
   * ```
   * // private function; not part of the public API — formats the profile error line
   * failure(new Error("bad colour"));
   * ```
   */
  function failure(err: unknown): string {
    return t("settings.profile.failed", { message: err instanceof Error ? err.message : String(err) });
  }

  /**
   * Saves colour and pronouns together, then shows them on this client's roster.
   * @param e The form's submit event; only `preventDefault` is used.
   * @returns Resolves once the outcome is shown; never rejects.
   * @example
   * ```
   * // private function; not part of the public API — wired to the form's onsubmit
   * declare const submitEvent: SubmitEvent;
   * await save(submitEvent);
   * ```
   */
  async function save(e: SubmitEvent): Promise<void> {
    e.preventDefault();
    error = null;
    const trimmed = pronouns.trim();
    busy = true;
    try {
      await updateProfile({ colour, pronouns: trimmed === "" ? null : trimmed });
    } catch (err) {
      error = failure(err);
      busy = false;
      return;
    }
    ctx.memberProfiles.set(ctx.selfId, { ...stored, colour, pronouns: trimmed === "" ? null : trimmed });
    colourDraft = undefined;
    pronounsDraft = undefined;
    busy = false;
  }

  /**
   * Uploads the chosen image as the avatar and shows it. The roster URL gains a cache-busting
   * query so the browser refetches the replaced image.
   * @param e The file input's change event.
   * @returns Resolves once the outcome is shown; never rejects.
   * @example
   * ```
   * // private function; not part of the public API — wired to the avatar file input
   * declare const changeEvent: Event;
   * await pickAvatar(changeEvent);
   * ```
   */
  async function pickAvatar(e: Event): Promise<void> {
    const input = e.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;
    error = null;
    busy = true;
    try {
      await uploadAvatar(file);
    } catch (err) {
      error = failure(err);
      busy = false;
      input.value = "";
      return;
    }
    const url = avatarUrl(ctx.world, ctx.selfId, true);
    ctx.memberProfiles.set(ctx.selfId, { ...stored, avatar: `${url}?v=${Date.now()}` });
    input.value = "";
    busy = false;
  }

  /**
   * Clears the avatar and drops it from this client's roster.
   * @returns Resolves once the outcome is shown; never rejects.
   * @example
   * ```
   * // private function; not part of the public API — wired to the remove-avatar button
   * await removeAvatar();
   * ```
   */
  async function removeAvatar(): Promise<void> {
    error = null;
    busy = true;
    try {
      await deleteAvatar();
    } catch (err) {
      error = failure(err);
      busy = false;
      return;
    }
    ctx.memberProfiles.set(ctx.selfId, { ...stored, avatar: null });
    busy = false;
  }
</script>

<section class="profile">
  <h3>{t("settings.profile.title")}</h3>
  <div class="avatar-row">
    {#if stored.avatar}
      <img class="avatar-preview" src={stored.avatar} alt={t("settings.profile.avatarAlt")} />
    {/if}
    <label>
      {t("settings.profile.avatar")}
      <input type="file" accept="image/png,image/jpeg,image/webp" disabled={busy} onchange={pickAvatar} />
    </label>
    {#if stored.avatar}
      <button type="button" disabled={busy} onclick={removeAvatar}>{t("settings.profile.avatarRemove")}</button>
    {/if}
  </div>
  <form onsubmit={save}>
    <div class="colour-row">
      <label>
        {t("settings.profile.colour")}
        <input
          type="color"
          value={colour ?? PICKER_PLACEHOLDER}
          oninput={(e) => (colourDraft = e.currentTarget.value)}
          onchange={(e) => (colourDraft = e.currentTarget.value)}
        />
      </label>
      <button type="button" disabled={colour === null} onclick={() => (colourDraft = null)}>
        {t("settings.profile.colourClear")}
      </button>
    </div>
    <label>
      {t("settings.profile.pronouns")}
      <input
        type="text"
        maxlength={MAX_PRONOUNS_CHARS}
        value={pronouns}
        oninput={(e) => (pronounsDraft = e.currentTarget.value)}
      />
    </label>
    <button type="submit" disabled={busy}>{t("settings.profile.save")}</button>
  </form>
  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}
</section>

<style lang="scss">
  .profile {
    display: grid;
    gap: var(--space-2);
  }
  form,
  .avatar-row,
  .colour-row {
    display: grid;
    gap: var(--space-2);
    justify-items: start;
  }
  .avatar-preview {
    width: 4rem;
    height: 4rem;
    border-radius: 999px;
    object-fit: cover;
  }
  .error {
    color: var(--danger);
  }
  input,
  button {
    min-height: 32px;
  }
  input[type="color"] {
    min-width: 48px;
  }
  @media (pointer: coarse) {
    input,
    button {
      min-height: var(--input-height-coarse);
    }
  }
</style>
