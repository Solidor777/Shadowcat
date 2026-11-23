<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import type { CombatantEngine, CombatantGroupEngine, WireDocument } from "@shadowcat/core";
  import type { Row } from "./model";

  /** Shape of an `Actor`-kind combatant's `engine.kind` narrowed cast, read for its
   * `token_id`. */
  type ActorKindShape = CombatantEngine & {
    /** The narrowed `CombatantKind` discriminant. */
    kind: {
      /** The kind's own discriminant literal. */
      type: "actor";
      /** The token this combatant tracks, when the row names one. */
      token_id: string | null;
    };
  };

  /** AddCombatants props. */
  interface Props {
    /** The combat to add combatants/events to. */
    combatId: string;
    /** The combat's current rows — used to filter out already-added tokens. */
    rows: Row[];
    /** The combat's `combatant-group` documents, offered by the group picker. */
    groups: WireDocument[];
    /** Whether the whole-session pause freezes this user's combat controls; disables every
     * control here, since each one writes combat documents. */
    frozen?: boolean;
  }
  const { combatId, rows, groups, frozen = false }: Props = $props();

  const ctx = getAppContext();

  const alreadyAdded = $derived.by((): Set<string> => {
    const ids = new Set<string>();
    for (const r of rows) {
      if (r.kind === "actor") {
        const tokenId = (r.doc.engine as ActorKindShape).kind.token_id;
        if (tokenId) ids.add(tokenId);
      }
    }
    return ids;
  });

  const selectableTokenIds = $derived.by((): string[] => [...ctx.tokenSelection.ids].filter((id) => !alreadyAdded.has(id)));

  let hiddenOnAdd = $state(false);
  /** The group picker value: `""` none, `"new"` create one, else an existing group id. */
  let groupChoice = $state("");
  let newGroupName = $state("");
  let newGroupColor = $state("#888888");

  /**
   * Adds every currently-selected, not-yet-added token as a combatant of `combatId`, each
   * hidden per the `hiddenOnAdd` toggle and joined to the picked group. The `"new"` choice
   * creates the group first (a blank name is refused) so every added combatant shares it.
   * @example
   * ```
   * // private function; not part of the public API — invoked from the "add selected" button
   * addSelected();
   * ```
   */
  function addSelected(): void {
    let group: string | undefined;
    if (groupChoice === "new") {
      const name = newGroupName.trim();
      if (!name) return;
      group = ctx.combat.createGroup(combatId, name, newGroupColor);
      groupChoice = group;
      newGroupName = "";
    } else if (groupChoice !== "") {
      group = groupChoice;
    }
    ctx.combat.addCombatants(
      combatId,
      selectableTokenIds.map((tokenId) => ({ tokenId, hidden: hiddenOnAdd, group })),
    );
  }

  let showEventForm = $state(false);
  let eventName = $state("");
  let eventLifespan = $state("");
  let eventMessage = $state("");
  let eventHidden = $state(false);

  /**
   * Authors a one-off `Event` combatant from the form fields, then resets the form and hides
   * it. A blank name is refused (no-op); a blank/non-finite lifespan or message is authored as
   * `null` rather than a stray empty string.
   * @example
   * ```
   * // private function; not part of the public API — invoked from the event form's submit
   * addEvent();
   * ```
   */
  function addEvent(): void {
    const name = eventName.trim();
    if (!name) return;
    const lifespan = eventLifespan.trim() === "" ? null : Number(eventLifespan);
    ctx.combat.addEvent(combatId, {
      name,
      lifespan: lifespan !== null && Number.isFinite(lifespan) ? lifespan : null,
      message: eventMessage.trim() === "" ? null : eventMessage,
      hidden: eventHidden,
    });
    eventName = "";
    eventLifespan = "";
    eventMessage = "";
    eventHidden = false;
    showEventForm = false;
  }
</script>

<fieldset class="add-combatants" disabled={frozen}>
  <label>
    <input type="checkbox" bind:checked={hiddenOnAdd} />
    {ctx.t("combatTracker.hidden")}
  </label>
  <label>
    {ctx.t("combatTracker.group")}
    <select aria-label={ctx.t("combatTracker.group")} data-testid="combat-tracker:group-picker" bind:value={groupChoice}>
      <option value="">{ctx.t("combatTracker.groupNone")}</option>
      {#each groups as g (g.id)}
        <option value={g.id}>{(g.engine as CombatantGroupEngine).name}</option>
      {/each}
      <option value="new">{ctx.t("combatTracker.groupNew")}</option>
    </select>
  </label>
  {#if groupChoice === "new"}
    <input type="text" aria-label={ctx.t("combatTracker.groupName")} bind:value={newGroupName} />
    <input type="color" aria-label={ctx.t("combatTracker.groupColor")} bind:value={newGroupColor} />
  {/if}
  <button type="button" data-testid="combat-tracker:add-selected" disabled={selectableTokenIds.length === 0} onclick={addSelected}>
    {ctx.plural("combatTracker.addSelected", selectableTokenIds.length, { n: selectableTokenIds.length })}
  </button>

  {#if showEventForm}
    <form onsubmit={(e) => { e.preventDefault(); addEvent(); }}>
      <label>
        {ctx.t("combatTracker.eventName")}
        <input type="text" aria-label={ctx.t("combatTracker.eventName")} bind:value={eventName} required />
      </label>
      <label>
        {ctx.t("combatTracker.eventLifespan")}
        <input type="number" aria-label={ctx.t("combatTracker.eventLifespan")} value={eventLifespan}
          oninput={(e) => (eventLifespan = (e.currentTarget as HTMLInputElement).value)} />
      </label>
      <label>
        {ctx.t("combatTracker.eventMessage")}
        <input type="text" aria-label={ctx.t("combatTracker.eventMessage")} bind:value={eventMessage} />
      </label>
      <label>
        <input type="checkbox" bind:checked={eventHidden} />
        {ctx.t("combatTracker.hidden")}
      </label>
      <button type="submit" data-testid="combat-tracker:add-event">{ctx.t("combatTracker.addEvent")}</button>
    </form>
  {:else}
    <button type="button" onclick={() => (showEventForm = true)}>{ctx.t("combatTracker.addEvent")}</button>
  {/if}
</fieldset>

<style lang="scss">
  /* A fieldset carries the native disabled-subtree behavior; strip its default frame so it lays
   * out exactly like the plain container it replaces. */
  .add-combatants {
    border: 0;
    margin: 0;
    padding: 0;
    min-inline-size: 0;
  }
</style>
