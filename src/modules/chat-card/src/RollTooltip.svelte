<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import type { RollOutcome } from "@shadowcat/core";

  let { outcome }: { outcome: RollOutcome } = $props();

  const ctx = getAppContext();
  const t = ctx.t;

  // Stable per-instance id (same convention as LauncherMenu.svelte's menuId): a message
  // can carry multiple inline rolls and many MessageCards mount simultaneously, so a
  // hardcoded id would collide across on-screen instances.
  const uid = $props.id();
  const popoverId = `roll-tooltip-popover-${uid}`;

  let open = $state(false);

  function show(): void {
    open = true;
  }
  function hide(): void {
    open = false;
  }

  /** iOS Safari moves neither focus nor mouseenter on tap, so hover/focus alone leave the
   * popover unreachable on touch. Hover-capable (mouse) devices already get open/close from
   * hover/focus; toggling here too would immediately re-close a hover-just-opened popover
   * (mouseenter fires before click). Gated on `(hover: hover)`, the same media query already
   * used for touch-affordance decisions elsewhere in this module family. */
  function onClick(): void {
    if (window.matchMedia("(hover: hover)").matches) return;
    open = !open;
  }

  /** WAI-ARIA `tooltip` pattern: Escape dismisses without moving focus off the trigger. */
  function onKeydown(event: KeyboardEvent): void {
    if (event.key === "Escape") {
      event.stopPropagation();
      hide();
    }
  }

  // A hover-opened (not focused) popover has no keydown target to catch Escape on, since
  // mousemove/mouseenter never focuses the trigger; a document-level listener while open
  // covers that case (the focused case is also handled redundantly by onKeydown above,
  // which additionally stops propagation).
  $effect(() => {
    if (!open) return;
    function onDocKeydown(event: KeyboardEvent): void {
      if (event.key === "Escape") hide();
    }
    document.addEventListener("keydown", onDocKeydown);
    return () => document.removeEventListener("keydown", onDocKeydown);
  });
</script>

<span class="roll-tooltip">
  <button
    type="button"
    class="roll-tooltip-trigger"
    aria-label={t("chat.roll.details")}
    aria-describedby={open ? popoverId : undefined}
    onfocus={show}
    onblur={hide}
    onmouseenter={show}
    onmouseleave={hide}
    onclick={onClick}
    onkeydown={onKeydown}
  >
    {outcome.successes ?? outcome.total}
  </button>
  {#if open}
    <div role="tooltip" id={popoverId} class="roll-tooltip-popover">
      <table>
        <tbody>
          {#each outcome.records as r, i (i)}
            <tr data-dropped={!r.kept}>
              <td class="value">{r.value}</td>
              {#if r.label}<td class="label">{r.label}</td>{/if}
              {#if !r.kept}<td class="dropped-tag">{t("chat.roll.dropped")}</td>{/if}
            </tr>
          {/each}
          {#each outcome.labeled_consts as c, i (i)}
            <tr>
              <td class="value">{c.value}</td>
              {#if c.label}<td class="label">{c.label}</td>{/if}
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
</span>

<style lang="scss">
  .roll-tooltip {
    position: relative;
    display: inline-flex;
  }
  .roll-tooltip-trigger {
    position: relative;
    display: inline-flex;
    padding: 0 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    font-weight: 700;
    background: none;
    color: inherit;
    font: inherit;
    cursor: pointer;
    align-items: center;
    justify-content: center;
  }
  // Touch floor via invisible hit-slop rather than a visible min-height/min-width: this
  // chip sits inline in running message text (unlike .roll-btn/.actions button, standalone
  // controls below the message), so inflating its own box would balloon line-height around
  // every inline roll result. An absolutely-positioned pseudo-element is out of flow, so it
  // widens the tap target to 44px without affecting text layout.
  .roll-tooltip-trigger::after {
    content: "";
    position: absolute;
    top: 50%;
    left: 50%;
    width: 44px;
    height: 44px;
    transform: translate(-50%, -50%);
  }
  .roll-tooltip-trigger:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .roll-tooltip-popover {
    position: absolute;
    z-index: 10;
    top: 100%;
    left: 0;
    margin-top: 4px;
    padding: var(--space-1);
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    background: var(--surface, #fff);
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.2);
    font-size: 0.9em;
    white-space: nowrap;
  }
  .roll-tooltip-popover table {
    border-collapse: collapse;
  }
  .roll-tooltip-popover td {
    padding: 0 var(--space-1) 0 0;
  }
  tr[data-dropped="true"] {
    opacity: 0.5;
    text-decoration: line-through;
  }
  .dropped-tag {
    font-size: 0.85em;
    text-decoration: none;
    opacity: 0.8;
  }
</style>
