<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import type { RollOutcome } from "@shadowcat/core";

  let { outcome }: { outcome: RollOutcome } = $props();

  const ctx = getAppContext();
  const t = ctx.t;

  let open = $state(false);

  function show(): void {
    open = true;
  }
  function hide(): void {
    open = false;
  }

  /** WAI-ARIA `tooltip` pattern: Escape dismisses without moving focus off the trigger. */
  function onKeydown(event: KeyboardEvent): void {
    if (event.key === "Escape") {
      event.stopPropagation();
      hide();
    }
  }
</script>

<span class="roll-tooltip">
  <button
    type="button"
    class="roll-tooltip-trigger"
    aria-label={t("chat.roll.details")}
    aria-describedby={open ? "roll-tooltip-popover" : undefined}
    onfocus={show}
    onblur={hide}
    onmouseenter={show}
    onmouseleave={hide}
    onkeydown={onKeydown}
  >
    {outcome.successes ?? outcome.total}
  </button>
  {#if open}
    <div role="tooltip" id="roll-tooltip-popover" class="roll-tooltip-popover">
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
    display: inline-flex;
    padding: 0 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    font-weight: 700;
    background: none;
    color: inherit;
    font: inherit;
    cursor: pointer;
    // Touch floor: this chip doubles as the popover trigger, so it must clear the
    // 44px minimum even though it renders as a small inline pill.
    min-height: 44px;
    min-width: 44px;
    align-items: center;
    justify-content: center;
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
