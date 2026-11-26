<script module lang="ts">
  /** Props of `UserMultiSelect`. Exported from the module block so the component's script is a
   * module: a script with no import or export is global, and its `onchange` prop would then
   * collide with the global `Window.onchange`. */
  export interface UserMultiSelectProps {
    /** userId -> display name for every pickable member (`AppContext.members`). */
    members: Map<string, string>;
    /** The currently selected user ids; the checkbox state is derived from this. */
    selected: readonly string[];
    /** The accessible name of the group. */
    label: string;
    /** Called with the full next selection (existing order, a newly checked id appended) when a checkbox toggles. */
    onchange: (next: string[]) => void;
  }
</script>

<script lang="ts">
  // Generic multi-user picker: one checkbox per world member, returning the selected user ids.
  // It holds no state of its own, so the caller decides what a selection means (a per-user
  // permission override list, a recipient list).
  let { members, selected, label, onchange }: UserMultiSelectProps = $props();

  const selectedSet = $derived(new Set(selected));
  const entries = $derived([...members.entries()]);

  /**
   * Computes the next selection after one member's checkbox toggled and reports it.
   * @param id The member whose checkbox toggled.
   * @param checked Whether the checkbox is now checked.
   * @example
   * ```
   * // private helper; not part of the public API — wired to each checkbox's change event
   * toggle("user-1", true);
   * ```
   */
  function toggle(id: string, checked: boolean): void {
    onchange(checked ? [...selected.filter((s) => s !== id), id] : selected.filter((s) => s !== id));
  }
</script>

<fieldset class="user-multi-select" aria-label={label}>
  <legend>{label}</legend>
  {#each entries as [id, username] (id)}
    <label class="member">
      <input type="checkbox" data-testid="user-multi-select-option" data-user-id={id} checked={selectedSet.has(id)}
        onchange={(e) => toggle(id, e.currentTarget.checked)} />
      {username}
    </label>
  {/each}
</fieldset>

<style lang="scss">
  .user-multi-select { border: 1px solid var(--border); border-radius: var(--radius-1); padding: var(--space-1); display: flex; flex-direction: column; }
  .member { display: flex; align-items: center; gap: var(--space-1); min-height: 44px; }
  .member input { min-width: 24px; min-height: 24px; }
</style>
