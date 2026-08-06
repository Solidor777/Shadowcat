<script lang="ts">
  // #region read-actors
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext, setField } from "@shadowcat/ui-kit";
  import { getPointer, type WireDocument } from "@shadowcat/core";
  import { rollInitiative, sortEntries, type Entry } from "./index";

  const ctx = getAppContext();

  // ctx.documents is a plain-callback store, not a rune: every $derived reading
  // it must subscribe itself or it freezes at first read (see ActorSheet's
  // createSubscriber pattern — same implicit coupling).
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));
  const actors = $derived.by((): WireDocument[] => {
    subscribe();
    return ctx.documents.query("actor");
  });
  // #endregion read-actors

  let entries = $state<Entry[]>([]);
  let turn = $state(0);
  const current = $derived(entries[turn]);

  // #region write-initiative
  /** Roll for one actor: track it locally, then persist the score onto the
   * actor's opaque `system` band ONLY if `ctx.canEdit` allows it — a denied
   * write stays local-only, with no error. Persisting uses `setField`'s OCC
   * contract — see src/client/ui-kit/src/sheetEdit.ts:4-10 for the pre-image
   * invariant this call must satisfy; not restated here to avoid a second,
   * driftable copy.
   * @param actor - The actor document to roll initiative for.
   * @example
   * ```
   * // private function; not part of the public API — invoked only from this
   * // component's "Roll" button onclick handler below
   * roll(actor);
   * ```
   */
  function roll(actor: WireDocument): void {
    const initiative = rollInitiative(() => Math.random());
    entries = sortEntries([
      ...entries.filter((e) => e.actorId !== actor.id),
      { actorId: actor.id, name: actor.name ?? "Unknown", initiative },
    ]);
    turn = 0;
    const path = "/system/initiative";
    if (ctx.canEdit(actor, path)) {
      setField(ctx, actor.id, path, getPointer(actor, path), initiative);
    }
  }
  // #endregion write-initiative

  /** Advance the turn pointer, wrapping at the end of the round.
   * @example
   * ```
   * // private function; not part of the public API — invoked only from this
   * // component's "Next turn" button onclick handler below
   * next();
   * ```
   */
  function next(): void {
    if (entries.length > 0) turn = (turn + 1) % entries.length;
  }
</script>

<div class="initiative">
  <h3>Initiative</h3>
  <ul>
    {#each actors as actor (actor.id)}
      <li>
        <span>{actor.name ?? "Unknown"}</span>
        <button type="button" onclick={() => roll(actor)}>Roll</button>
      </li>
    {/each}
  </ul>
  {#if entries.length > 0}
    <ol>
      {#each entries as e, i (e.actorId)}
        <li class:active={i === turn}>{e.name} — {e.initiative}</li>
      {/each}
    </ol>
    <p>Current: {current?.name}</p>
    <button type="button" onclick={next}>Next turn</button>
  {/if}
</div>

<style>
  .initiative { padding: 0.5rem; }
  /* Touch-sized targets (cross-platform UI invariant). */
  button { min-height: 44px; min-width: 44px; }
  .active { font-weight: bold; }
</style>
