<script lang="ts">
  // Generic editor for a document's `PermissionSet`: the `default` role, the per-user role
  // overrides (`users`) and, when enabled, the `gm_role` cap. It knows nothing about the
  // document type. Every edit is one field-path Update whose `old` is the RAW stored value, and
  // the whole editor is absent unless the viewer may write `/permissions` (advisory; the server
  // re-checks `core:edit_permissions`).
  import { createSubscriber } from "svelte/reactivity";
  import type { WireDocument, WirePermissionSet } from "@shadowcat/core";
  import { getAppContext } from "./appContext";
  import { setField, unsetField } from "./sheetEdit";
  import UserMultiSelect from "./UserMultiSelect.svelte";

  /** A role a `PermissionSet` field can hold. */
  type Role = WirePermissionSet["default"];

  let { docId, gmRole = false }: {
    /** The document whose permissions this edits. */
    docId: string;
    /** Whether to offer the `gm_role` cap. Off by default: capping the GM on a document can lock
     * the GM out of undoing it, so a caller opts in only where that is intended. */
    gmRole?: boolean;
  } = $props();

  const ctx = getAppContext();
  const t = ctx.t;
  const ROLES: readonly Role[] = ["none", "observer", "owner"];

  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));
  const doc = $derived.by((): WireDocument | undefined => {
    subscribe();
    return ctx.documents.get(docId);
  });
  const editable = $derived(!!doc && ctx.canEdit(doc, "/permissions"));
  const overrideIds = $derived(doc ? Object.keys(doc.permissions.users) : []);

  /**
   * Narrows a `<select>` value to a role.
   * @param value The raw option value.
   * @returns The matching role; unknown values read as `none`, the fail-closed role.
   * @example
   * ```
   * // private helper; not part of the public API
   * toRole("owner"); // "owner"
   * ```
   */
  function toRole(value: string): Role {
    return value === "owner" || value === "observer" ? value : "none";
  }

  /**
   * Writes `permissions.default`.
   * @param value The new default role.
   * @example
   * ```
   * // private helper; not part of the public API — wired to the default select
   * setDefault("observer");
   * ```
   */
  function setDefault(value: Role): void {
    if (doc) setField(ctx, docId, "/permissions/default", doc.permissions.default, value);
  }

  /**
   * Writes one user's override role.
   * @param userId The user whose override changes.
   * @param value The new role for that user.
   * @example
   * ```
   * // private helper; not part of the public API — wired to each override's role select
   * setUserRole("user-1", "owner");
   * ```
   */
  function setUserRole(userId: string, value: Role): void {
    if (doc) setField(ctx, docId, `/permissions/users/${userId}`, doc.permissions.users[userId], value);
  }

  /**
   * Applies a new override selection from the picker: a newly selected user gets an `observer`
   * override (an absent key, so the pre-image is `undefined`), a deselected user's key is removed.
   * @param next The full next set of user ids holding an override.
   * @example
   * ```
   * // private helper; not part of the public API — wired to the picker's change callback
   * setOverrides(["user-1"]);
   * ```
   */
  function setOverrides(next: string[]): void {
    if (!doc) return;
    const stored = doc.permissions.users;
    for (const id of next) if (!(id in stored)) setField(ctx, docId, `/permissions/users/${id}`, undefined, "observer");
    for (const id of Object.keys(stored)) if (!next.includes(id)) unsetField(ctx, docId, `/permissions/users/${id}`, stored[id]);
  }

  /**
   * Writes the `gm_role` cap; an empty value clears it back to the unconditional GM access.
   * @param value The selected option value, `""` for no cap.
   * @example
   * ```
   * // private helper; not part of the public API — wired to the GM role select
   * setGmRole("");
   * ```
   */
  function setGmRole(value: string): void {
    if (doc) setField(ctx, docId, "/permissions/gm_role", doc.permissions.gm_role, value === "" ? null : toRole(value));
  }
</script>

{#if doc && editable}
  <div class="permissions-editor" data-testid="permissions-editor">
    <label>{t("permissions.default")}
      <select data-testid="permissions-default" value={doc.permissions.default}
        onchange={(e) => setDefault(toRole(e.currentTarget.value))}>
        {#each ROLES as role (role)}
          <option value={role}>{t(`permissions.role.${role}`)}</option>
        {/each}
      </select></label>

    <UserMultiSelect members={ctx.members} selected={overrideIds} label={t("permissions.users")} onchange={setOverrides} />

    {#each overrideIds as userId (userId)}
      <label>{ctx.members.get(userId) ?? userId}
        <select data-testid="permissions-user-role" data-user-id={userId} value={doc.permissions.users[userId]}
          onchange={(e) => setUserRole(userId, toRole(e.currentTarget.value))}>
          {#each ROLES as role (role)}
            <option value={role}>{t(`permissions.role.${role}`)}</option>
          {/each}
        </select></label>
    {/each}

    {#if gmRole}
      <label>{t("permissions.gmRole")}
        <select data-testid="permissions-gm-role" value={doc.permissions.gm_role ?? ""}
          onchange={(e) => setGmRole(e.currentTarget.value)}>
          <option value="">{t("permissions.gmRoleUncapped")}</option>
          {#each ROLES as role (role)}
            <option value={role}>{t(`permissions.role.${role}`)}</option>
          {/each}
        </select></label>
    {/if}
  </div>
{/if}

<style lang="scss">
  .permissions-editor { display: flex; flex-direction: column; gap: var(--space-1); }
  label { display: flex; flex-direction: column; gap: 2px; }
  select { min-height: 44px; }
</style>
