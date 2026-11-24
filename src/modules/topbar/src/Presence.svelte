<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";

  const ctx = getAppContext();
  const t = ctx.t;

  // A reactive SvelteMap (userId -> username), populated for every role. `WorldSession`
  // refills it from the roster read on each WS (re)connect Welcome and adds a member seated
  // mid-session from the server's `member_profile` frame. The map is mutated in place, never
  // reassigned, so reading it via $derived repaints on either update.
  //
  // `ctx.presence` overlays a live online/offline dot on each badge. A member with no entry
  // there is unknown (the roster read has not landed yet), so the badge carries neither state
  // class and no suffix.
  const roster = $derived(
    [...ctx.members.entries()].map(([id, name]) => {
      const online = ctx.presence.get(id);
      const state = online === undefined ? "" : online ? t("topbar.online") : t("topbar.offline");
      const profile = ctx.memberProfiles.get(id);
      const colour = profile?.colour ?? null;
      // Pronouns follow the name in both the hover title and the accessible label, so every
      // member's client shows them once the server's `member_profile` frame lands.
      const display = profile?.pronouns ? `${name} (${profile.pronouns})` : name;
      return {
        id,
        title: display,
        online,
        colour,
        name,
        avatar: profile?.avatar ?? null,
        label: state ? `${display} (${state})` : display,
      };
    }),
  );

  /**
   * Readable glyph colour for a badge filled with `hex`: black on a light fill, white on a dark
   * one, by the WCAG relative-luminance threshold (sRGB channels linearized, 0.179 is where black
   * and white text have equal contrast). Source: WCAG 2.x relative luminance definition.
   * @param hex A `#rrggbb` fill colour.
   * @returns `"#000000"` or `"#ffffff"`.
   * @example
   * ```
   * // private function; not part of the public API — picks the glyph colour for a coloured badge
   * glyphOn("#336699");
   * ```
   */
  function glyphOn(hex: string): string {
    const lin = (i: number): number => {
      const c = parseInt(hex.slice(i, i + 2), 16) / 255;
      return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
    };
    const luminance = 0.2126 * lin(1) + 0.7152 * lin(3) + 0.0722 * lin(5);
    return luminance > 0.179 ? "#000000" : "#ffffff";
  }

  /**
   * Single-character badge glyph for a member name: the trimmed name's first
   * character, uppercased, or `"?"` for an empty/whitespace-only name.
   * @param name The member's display name.
   * @returns A one-character (or `"?"`) badge glyph.
   * @example
   * ```
   * // private function; not part of the public API — used to render each presence badge
   * initial("Zara");
   * ```
   */
  function initial(name: string): string {
    return name.trim().charAt(0).toUpperCase() || "?";
  }
</script>

<div class="sc-presence" role="group" aria-label={t("topbar.presence")} data-testid="presence">
  {#each roster as m (m.id)}
    <span
      class="sc-presence-badge"
      title={m.title}
      aria-label={m.label}
      style:background={m.colour ?? undefined}
      style:color={m.colour ? glyphOn(m.colour) : undefined}
      data-testid="presence-{m.id}">{#if m.avatar}<img
          class="sc-presence-avatar"
          src={m.avatar}
          alt=""
          data-testid="presence-avatar-{m.id}" />{:else}{initial(m.name)}{/if}<span
        class="sc-presence-dot"
        class:online={m.online === true}
        class:offline={m.online === false}
        aria-hidden="true"
        data-testid="presence-dot-{m.id}"></span></span>
  {/each}
</div>

<style lang="scss">
  .sc-presence {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    overflow: hidden;
  }
  .sc-presence-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.5rem;
    height: 1.5rem;
    border-radius: 999px;
    background: var(--surface-overlay);
    border: 1px solid var(--border);
    color: var(--text-primary);
    font-size: 0.75rem;
    line-height: 1;
    flex: 0 0 auto;
    position: relative;
  }
  .sc-presence-avatar {
    width: 100%;
    height: 100%;
    border-radius: 999px;
    object-fit: cover;
  }
  // The dot paints only when the member's state is known (`.online` or `.offline`).
  .sc-presence-dot {
    position: absolute;
    right: -1px;
    bottom: -1px;
    width: 0.5rem;
    height: 0.5rem;
    border-radius: 999px;
    border: 1px solid var(--surface-overlay);
    display: none;
    &.online {
      display: block;
      background: var(--success);
    }
    &.offline {
      display: block;
      background: var(--text-muted);
    }
  }
</style>
