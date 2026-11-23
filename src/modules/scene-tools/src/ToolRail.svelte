<script lang="ts">
  import { createSubscriber } from "svelte/reactivity";
  import { getAppContext, sizeClass, LightEmissionEditor, escapeCoordinator, ESCAPE_PRIORITY } from "@shadowcat/ui-kit";
  import { resolveSceneSettings, sessionFreezes, actorDisplayName, resolveTokenActor, templateToRegionShape, ownerFloorApplies, buildUpdate, SCENE_TOOL_CONTRACT, type WireDocument, type LightEngine, type TokenEngine, type WallEngine, type WallProximity, type WallSense,type RegionEngine, type RegionAnchor, type DrawingEngine, type TemplateEngine, type RegionTrigger, type TriggerEvent, type NoticeAudience, type SceneEngine, type ElevationBand, type SceneToolMeta, type AmbientLightMode } from "@shadowcat/core";
  import { ToolController, type HostToolContext, type ToolId, type DrawMode, type TemplateMode, type RegionShapeMode, type RegionBehaviorMode, type ViewedLevelBand } from "./controller.svelte";
  import AssetPicker from "./AssetPicker.svelte";
  import RegionTriggerTeleportEditor from "./RegionTriggerTeleportEditor.svelte";
  import { MAX_OVERLAY_DISPLAY_CHARS, type SnapMode } from "@shadowcat/render";

  const ctx = getAppContext();

  /**
   * Resolves the `SceneLevel` `ctx.viewedLevel` names on the viewed scene, or `null` (no scene,
   * no viewed level, or the level id no longer names a declared level).
   * @returns The resolved `SceneLevel`, or `null`.
   * @example
   * ```
   * // private helper; not part of the public API — read by viewedLevelBand/viewedLevelBottom
   * resolvedViewedLevel();
   * ```
   */
  function resolvedViewedLevel(): ViewedLevelBand | null {
    const sceneId = ctx.viewedSceneId;
    const level = ctx.viewedLevel;
    if (sceneId === null || level === null) return null;
    const scene = ctx.documents.query("scene").find((s) => s.id === sceneId);
    const levels = (scene?.engine as SceneEngine | undefined)?.levels ?? [];
    const found = levels.find((l) => l.id === level);
    return found ? { bottom: found.bottom, top: found.top } : null;
  }

  // The controller is fixed per ToolRail instance; capturing the context once is intended.
  // `satisfies HostToolContext` makes every AppContext-supplied ToolContext member mandatory
  // here, so a seam the host has can never be left out of this literal without a type error.
  // The two viewedLevel* fields below are ToolContext-only (no AppContext counterpart), so they
  // sit OUTSIDE this checked literal — spread in below — rather than widening the check itself.
  // svelte-ignore state_referenced_locally
  const hostToolContext = {
    scene: ctx.scene,
    actorSelection: ctx.actorSelection,
    tokenSelection: ctx.tokenSelection,
    dispatchIntent: ctx.dispatchIntent,
    documents: ctx.documents,
    assets: ctx.assets,
    world: ctx.world,
    role: ctx.role,
    sendPing: ctx.sendPing,
    shareRuler: ctx.shareRuler,
    sendRulerUpdate: ctx.sendRulerUpdate,
    pathfind: ctx.pathfind,
    moveRequest: ctx.moveRequest,
    toggleDoor: ctx.toggleDoor,
    notify: ctx.notify,
    selfId: ctx.selfId,
    combat: ctx.combat,
    viewedSceneId: () => ctx.viewedSceneId,
    footprints: () => ctx.footprints,
    t: ctx.t,
    setGmViewedScene: ctx.setGmViewedScene,
  } satisfies HostToolContext;
  const controller = new ToolController({
    ...hostToolContext,
    viewedLevelBand: () => resolvedViewedLevel(),
    viewedLevelBottom: () => resolvedViewedLevel()?.bottom ?? null,
    movesFrozen: () => movesFrozen,
  });
  const t = ctx.t;
  // Authoring is GM-gated (the server is authoritative; this hides the controls).
  // Gating is PER TOOL, not per component: the controller is constructed for every user so
  // a player has an active tool at all — without one, a canvas drag falls through to camera pan.
  const isGm = ctx.role === "gm";

  // Compact: the rail renders as a horizontal bottom strip (core-ui repositions
  // it into the compact grid's bottom row); expanded: a vertical side rail.
  const compact = $derived(sizeClass() === "compact");

  // Reactive subscription mirrors GameSettingsPanel's registry-seed pattern: calling
  // subscribe() inside each $derived.by registers a reactive dependency on the document
  // store so the snap toggle re-resolves as the active scene's doc changes.
  const subscribe = createSubscriber((update) => ctx.documents.subscribe(update));
  const activeScene = $derived.by((): WireDocument | undefined => {
    subscribe();
    const vsid = ctx.viewedSceneId;
    return vsid ? ctx.documents.get(vsid) : undefined;
  });
  const snapToGrid = $derived.by((): boolean => {
    subscribe();
    return resolveSceneSettings(activeScene, ctx.documents).snapToGrid;
  });
  // The whole-session pause freezes a non-GM's token movement. The select and measure tools stay
  // usable (selection, doors, route previews); their move commits are what the controller
  // refuses, and their tooltip says why.
  const movesFrozen = $derived.by((): boolean => {
    subscribe();
    return sessionFreezes(ctx.documents, ctx.role);
  });
  /** The tools whose gestures commit token movement. */
  const MOVE_TOOLS: readonly ToolId[] = ["select", "measure"];

  // Contributed scene tools (SCENE_TOOL_CONTRACT): contribution changes are NOT document-store
  // changes, so this needs its own `ctx.contributions.subscribe` bridge, separate from the
  // document-store `subscribe` above.
  const subscribeContributions = createSubscriber((update) => ctx.contributions.subscribe(update));
  const sceneTools = $derived.by((): SceneToolMeta[] => {
    subscribeContributions();
    return ctx.contributions
      .contributionsFor(SCENE_TOOL_CONTRACT)
      .map((c) => c.sceneTool)
      .filter((m): m is SceneToolMeta => m !== undefined);
  });

  /** GM-authored scene-level snap toggle: writes the engine-owned
   * `/engine/snapToGrid` field on the active scene document (shared, not local UI state).
   * No-op with no active scene.
   * @example
   * ```
   * toggleSnap();
   * ```
   */
  function toggleSnap(): void {
    const scene = activeScene;
    if (!scene) return;
    // Reads the RAW stored value (not the resolved/defaulted `snapToGrid`) for optimistic-
    // concurrency `old`: the server's field-level conflict check compares against the actual
    // stored value at this path, which is only `null` while the field is genuinely absent.
    // Mirrors `commitMoves`'s GM branch convention (`eng?.x ?? null`).
    const rawSnap = (scene.engine as {
      /** The raw stored snap flag; absent (not merely falsy) means "use the derived default". */
      snapToGrid?: boolean;
    } | undefined)?.snapToGrid ?? null;
    ctx.dispatchIntent([buildUpdate(scene.id, [{ path: "/engine/snapToGrid", old: rawSnap, value: !snapToGrid }])]);
  }

  /** The snap modes in the order the mode button cycles through them. */
  const SNAP_MODE_CYCLE: SnapMode[] = ["center", "vertex", "edge"];

  /** The mode button's visible label for `mode`.
   * @param mode The snap mode to label.
   * @returns The localized label.
   * @example
   * ```
   * snapModeLabel("edge");
   * ```
   */
  function snapModeLabel(mode: SnapMode): string {
    switch (mode) {
      case "vertex": return t("tools.snapModeVertex");
      case "edge": return t("tools.snapModeEdge");
      case "center": return t("tools.snapModeCenter");
    }
  }

  /** Advances the session's snap-MODE preference to the next mode in the cell-center, vertex,
   * edge cycle. Client-local session state (`ctx.snapModePreference`), never a document write —
   * unlike `toggleSnap` above, nothing here is shared with other clients; `Stage` observes
   * the preference and re-pushes it to the engine's `setSnapMode` chokepoint.
   * @example
   * ```
   * toggleSnapMode();
   * ```
   */
  function toggleSnapMode(): void {
    const next = (SNAP_MODE_CYCLE.indexOf(ctx.snapModePreference.mode) + 1) % SNAP_MODE_CYCLE.length;
    ctx.snapModePreference.set(SNAP_MODE_CYCLE[next]);
  }

  /** `gmOnly` marks a tool that AUTHORS scene content (creates or edits a document other
   * than a token's own position). The three ungated tools each go through a path the server
   * already polices for a non-GM — but select/move does NOT go through the same path for every
   * role: `commitMoves` branches on `ctx.role`. A GM writes
   * `/engine/x,y` directly via `dispatchIntent` — an ordinary permission-checked `Update`, with
   * NO movement gate at all, since `Room::publish`'s non-GM position-refusal block
   * does not apply to a GM's own write. A non-GM's move is
   * request-only and never writes `/engine/x,y` directly: per selected token, `Pathfind` then
   * `MoveRequest` → `execute_move`'s per-cell wall/mask/footprint gate — the same mechanism the
   * measure tool's `commitRoute` uses. Measure previews via the per-requester-masked `Pathfind`
   * and commits a route the same request-only way (`MoveRequest`/`execute_move`); ping is the
   * rate-limited per-user relay.
   *
   * `place` in particular MUST stay `gmOnly`, for a reason that is not fully visible from this
   * file: `Room::publish`'s `Create` gate authorizes a
   * placed token's CENTER cell against the requester's visibility mask (matching
   * `movement_restriction` — no check at all under `Unrestricted`) but NEVER checks walls: a
   * placement is a single point, not a traversal, so there is no `line_traversal`/supercover call
   * to run a wall test against. A `Create` can
   * therefore still place a token behind/through a wall the
   * movement gate would otherwise block. The other server-side check on a player `Create` is
   * `apply_intent`'s `core:create` world-capability grant, which is world-CONFIGURABLE —
   * fail-closed by default, but a world that granted it would have a real wall-bypass placement
   * hole if this tool were ungated. Ungating an authoring tool therefore requires checking what
   * gates its op KIND, not just that some gate exists on the path. */
  const tools: {
    /** The tool id, passed to `controller.toggle`. */
    id: ToolId;
    /** The button's visible label and title (already localized via `t`). */
    label: string;
    /** Whether the tool authors content and must therefore hide from non-GMs (see the doc
     * comment above this array for the per-tool rationale). */
    gmOnly: boolean;
  }[] = [
    { id: "select", label: t("tools.select"), gmOnly: false },
    { id: "place", label: t("tools.place"), gmOnly: true },
    { id: "draw", label: t("tools.draw"), gmOnly: true },
    { id: "template", label: t("tools.template"), gmOnly: true },
    { id: "measure", label: t("tools.measure"), gmOnly: false },
    { id: "ping", label: t("tools.ping"), gmOnly: false },
    { id: "wall", label: t("tools.wall"), gmOnly: true },
    { id: "region", label: t("tools.region"), gmOnly: true },
    { id: "light", label: t("tools.light"), gmOnly: true },
  ];
  const visibleTools = tools.filter((tool) => isGm || !tool.gmOnly);
  // Speak-as + emote affordance gate: exactly one token selected and the current user may
  // plausibly act as it (GM, or the effective owner) — advisory only, mirroring the
  // client-side offer/server-reauthorizes split (the server enforces both `SendMessage`'s
  // `actor_owner` and the emote relay's effective-ownership gate). Reuses the subscriber
  // bridge already established above for `activeScene`/`snapToGrid`.
  const selectedSingleToken = $derived.by((): WireDocument | null => {
    subscribe();
    const ids = ctx.tokenSelection.ids;
    if (ids.size !== 1) return null;
    const [id] = ids;
    return ctx.documents.get(id) ?? null;
  });
  const canActAsSelected = $derived.by((): boolean => {
    subscribe();
    const tok = selectedSingleToken;
    if (!tok) return false;
    return ctx.role === "gm" || ownerFloorApplies(tok, ctx.selfId, ctx.documents);
  });

  /** The viewed scene's tokens this user effectively owns (`ownerFloorApplies`: direct owner, or
   * the linked actor's owner). The pan-to-owned button's subject and its enablement. */
  const ownedTokens = $derived.by((): WireDocument[] => {
    subscribe();
    const sceneId = ctx.viewedSceneId;
    if (sceneId === null) return [];
    return ctx.documents
      .query("token")
      .filter((tok) => tok.parent_id === sceneId && ownerFloorApplies(tok, ctx.selfId, ctx.documents));
  });

  /** Pans the camera to the average position of the user's owned tokens on the viewed scene
   * (a token's `engine.x`/`engine.y` is its center). A no-op with none; the button is disabled
   * then.
   * @example
   * ```
   * // wired to the "center on my tokens" button
   * panToOwnedTokens();
   * ```
   */
  function panToOwnedTokens(): void {
    const owned = ownedTokens;
    if (owned.length === 0) return;
    let x = 0;
    let y = 0;
    for (const tok of owned) {
      const eng = tok.engine as TokenEngine;
      x += eng.x;
      y += eng.y;
    }
    ctx.scene.panToPoint(x / owned.length, y / owned.length);
  }

  /** Sets the pending speak-as-token selection from the currently selected token (a no-op if
   * none is selected, guarded by `canActAsSelected` at the call site).
   * @example
   * ```
   * // internal; wired to the "speak as this token" button
   * speakAsSelectedToken();
   * ```
   */
  function speakAsSelectedToken(): void {
    const tok = selectedSingleToken;
    if (tok) ctx.speakAsToken.select(tok.id);
  }

  /** The palette's stock emote glyphs — every entry well under the server's per-emote byte
   * cap (16 bytes covers 1–4 emoji graphemes), so a palette click can never silent-drop. */
  const emoteChoices = ["😀", "😂", "😮", "😢", "😡", "👍", "❤️", "🎉", "🔥", "⚔️"];
  /** The free-input emote draft, cleared after a successful send. */
  let emoteDraft = $state("");
  /** Byte length of `s` (UTF-8) — the unit the server's emote bound is stated in.
   * @param s The emote draft to measure.
   * @returns The UTF-8 byte length of `s`.
   * @example
   * ```
   * emoteByteLength("😀"); // 4
   * ```
   */
  function emoteByteLength(s: string): number {
    return new TextEncoder().encode(s).length;
  }
  /** The draft is sendable when it is non-blank and within the server's byte bound —
   * client-advisory, so an over-long draft can't vanish into the server's silent drop. */
  const emoteDraftSendable = $derived(emoteDraft.trim() !== "" && emoteByteLength(emoteDraft.trim()) <= 16);

  /** Send an emote over the currently selected token (a no-op without an effectively-owned
   * selection — `canActAsSelected` guards the call sites). The server relays it back to all
   * members (incl. us), so the local overlay arrives via the emote listener like any other.
   * @param emote The emote glyph(s) to send.
   * @example
   * ```
   * // internal; wired to the emote palette buttons
   * sendEmoteAsSelected("😀");
   * ```
   */
  function sendEmoteAsSelected(emote: string): void {
    const tok = selectedSingleToken;
    if (!tok || !canActAsSelected) return;
    ctx.sendEmote(tok.id, emote);
  }

  /** Send the free-input draft and clear it (guarded by `emoteDraftSendable`).
   * @example
   * ```
   * // internal; wired to the emote send button
   * sendEmoteDraft();
   * ```
   */
  function sendEmoteDraft(): void {
    const draft = emoteDraft.trim();
    if (!emoteDraftSendable) return;
    sendEmoteAsSelected(draft);
    emoteDraft = "";
  }

  const drawModes: DrawMode[] = ["freehand", "rect", "ellipse", "line"];
  const templateModes: TemplateMode[] = ["circle", "cone", "rect", "line"];
  const regionShapeModes: RegionShapeMode[] = ["rect", "circle", "polygon"];
  const regionBehaviors: RegionBehaviorMode[] = ["terrain", "impassable", "arrest", "ambient_light"];
  const ambientLightModes: AmbientLightMode[] = ["darken", "brighten", "override"];

  // The open editor's target document, resolved live from the shared editing selection
  // (reactive: re-reads on every store commit, so the inputs reflect the server's state).
  const editingDoc = $derived.by((): WireDocument | null => {
    subscribe();
    const sel = controller.editingEntity;
    if (!sel) return null;
    const doc = ctx.documents.get(sel.id);
    // A deleted/elsewhere-moved doc drops the selection rather than editing a ghost.
    if (!doc || doc.doc_type !== sel.kind) return null;
    return doc;
  });
  // Keep the controller's selection honest when the target disappears.
  $effect(() => {
    if (controller.editingEntity && !editingDoc) controller.editingEntity = null;
  });

  /** Dispatch one field-level update against the entity open in the editor. `old` MUST be the
   * RAW stored value at `path` (the server's field-level optimistic-concurrency check compares
   * against it) — never a resolved/defaulted value. The standing raw-`old` convention.
   * @param path The engine-band JSON pointer being written.
   * @param oldRaw The raw stored value currently at `path` (`null` when absent).
   * @param value The new value to write.
   * @example
   * ```
   * editSelected("/engine/emission/enabled", true, false);
   * ```
   */
  function editSelected(path: string, oldRaw: unknown, value: unknown): void {
    const doc = editingDoc;
    if (!doc) return;
    ctx.dispatchIntent([buildUpdate(doc.id, [{ path, old: oldRaw, value }])]);
  }

  /** Delete the document open in the editor (full pre-image op) and clear the selection.
   * @example
   * ```
   * deleteSelected();
   * ```
   */
  function deleteSelected(): void {
    const doc = editingDoc;
    if (!doc) return;
    ctx.dispatchIntent([{ op: "delete", doc }]);
    controller.editingEntity = null;
  }

  /** Parse an elevation input's raw string: empty means "absent" (ground for a light, an
   * unbounded end for a wall band); a finite number is the value; anything else is ignored
   * (never dispatch a NaN).
   * @param raw The input's raw string value.
   * @returns The parsed elevation, `null` for absent, or `undefined` for "ignore this edit".
   * @example
   * ```
   * parseElevation("");    // null  (absent)
   * parseElevation("2.5"); // 2.5
   * parseElevation("abc"); // undefined — no write
   * ```
   */
  function parseElevation(raw: string): number | null | undefined {
    if (raw.trim() === "") return null;
    const n = Number(raw);
    return Number.isFinite(n) ? n : undefined;
  }

  /** Write the edited light's `/engine/elevation`. Ground (`0`/empty) normalizes to `null`
   * (canonical absent, matching `elevation_or_ground`'s None = 0 read); `old` is the raw
   * stored value.
   * @param raw The elevation input's raw string value.
   * @example
   * ```
   * editLightElevation("10"); // a light 10 units above the ground plane
   * ```
   */
  function editLightElevation(raw: string): void {
    const doc = editingDoc;
    if (!doc) return;
    const eng = doc.engine as LightEngine;
    const parsed = parseElevation(raw);
    if (parsed === undefined) return;
    const next = parsed === 0 ? null : parsed;
    const old = eng.elevation ?? null;
    if (next === old) return;
    editSelected("/engine/elevation", old, next);
  }

  /** One entry of the "Follows token" picker. */
  interface RegionCarrierOption {
    /** The token document id the option selects. */
    id: string;
    /** The option's visible text. */
    label: string;
  }

  /** The tokens on the edited region's own scene, as `{ id, label }` options for the "Follows
   * token" picker (a carrier must be a token on the region's scene). The label is the
   * redaction-safe display name (`actorDisplayName`), falling back to a short id.
   * @param region The region document open in the editor.
   * @returns The scene's tokens, in document order.
   * @example
   * ```
   * declare const region: WireDocument;
   * regionCarrierOptions(region); // [{ id: "t1", label: "Goblin" }]
   * ```
   */
  function regionCarrierOptions(region: WireDocument): RegionCarrierOption[] {
    return ctx.documents
      .query("token")
      .filter((tok) => tok.parent_id === region.parent_id)
      .map((tok) => ({ id: tok.id, label: actorDisplayName(resolveTokenActor(tok, ctx.documents) ?? {}, tok.id.slice(0, 8)) }));
  }

  /** Apply a "Follows token" edit to the open region: the carrier picker (`tokenId` set; an empty
   * string detaches) or one of the two flag checkboxes (`patch`). The flags are kept when the
   * carrier changes. The shape conversion lives in `ToolController.setRegionAnchor`.
   * @param eng The edited region's engine body (the current anchor is the base of the change).
   * @param tokenId The picked carrier id, `""` to detach, or `undefined` to keep the current one.
   * @param patch Flag changes to merge into the anchor.
   * @example
   * ```
   * declare const eng: RegionEngine;
   * editRegionAnchor(eng, "t1", {}); // attach to token t1
   * ```
   */
  function editRegionAnchor(eng: RegionEngine, tokenId: string | undefined, patch: Partial<RegionAnchor>): void {
    const doc = editingDoc;
    if (!doc) return;
    if (tokenId === "") {
      controller.setRegionAnchor(doc.id, null);
      return;
    }
    const token = tokenId ?? eng.anchor?.token;
    if (!token) return;
    controller.setRegionAnchor(doc.id, { include_carrier: false, follow_rotation: false, ...eng.anchor, ...patch, token });
  }

  /** The senses a region can occlude, in editor order; each is a `RegionOcclusion` key. */
  const OCCLUDE_SENSES = ["sight", "light", "sound"] as const;
  /** The translation key labelling each occlusion checkbox. */
  const OCCLUDE_LABEL = {
    sight: "tools.regionOccludesSight",
    light: "tools.regionOccludesLight",
    sound: "tools.regionOccludesSound",
  } as const;

  /** Write one sense flag of the edited region's `/engine/occludes` block, keeping the other
   * flags. Clearing the last ticked flag writes `null` (an all-false block is rejected by
   * `RegionEngine::validate`, so "blocks nothing" is stored as absent).
   * @param eng The edited region's engine body (the raw stored `occludes` is the write's `old`).
   * @param sense Which sense the checkbox toggles.
   * @param checked The checkbox's new state.
   * @example
   * ```
   * declare const eng: RegionEngine;
   * editRegionOcclusion(eng, "sight", true); // fog bank
   * ```
   */
  function editRegionOcclusion(eng: RegionEngine, sense: (typeof OCCLUDE_SENSES)[number], checked: boolean): void {
    const old = eng.occludes ?? null;
    const flags = { sight: false, light: false, sound: false, ...old, [sense]: checked };
    const next = flags.sight || flags.light || flags.sound ? flags : null;
    editSelected("/engine/occludes", old, next);
  }

  /** Write one end of an edited entity's `/engine/elevation` band, preserving the other end —
   * the shared body behind `editEntityElevation`, the one band editor every band-shaped kind
   * (wall, region, drawing, template) uses: they all carry the identical `Option<ElevationBand>`
   * shape. An emptied end is unbounded (`null`); when BOTH ends are unbounded the whole field
   * resolves to `null` (canonical "every elevation" — an absent band, matching
   * `band_contains`/`wall_occludes`).
   * @param old The raw stored band object (or `null`).
   * @param end Which band end this edit changes; the other end is carried forward unchanged.
   * @param raw The input's raw string value.
   * @returns The next band value, or `undefined` when `raw` doesn't parse (no write).
   * @example
   * ```
   * editElevationBand(null, "bottom", "2"); // { bottom: 2, top: null }
   * ```
   */
  function editElevationBand(
    old: ElevationBand | null,
    end: "bottom" | "top",
    raw: string,
  ): ElevationBand | null | undefined {
    const parsed = parseElevation(raw);
    if (parsed === undefined) return undefined;
    const bottom = end === "bottom" ? parsed : (old?.bottom ?? null);
    const top = end === "top" ? parsed : (old?.top ?? null);
    return bottom === null && top === null ? null : { bottom, top };
  }

  /** Write one end of the edited entity's `/engine/elevation` band, preserving the other end.
   * `old` is the raw stored band object (or `null`). One shared body for every band-shaped kind
   * (wall/region/drawing/template) — they all carry the identical `elevation: ElevationBand |
   * null` engine field, so a per-kind cast would only fork this decision, never change it.
   * @param end Which band end this edit changes; the other end is carried forward unchanged.
   * @param raw The input's raw string value.
   * @example
   * ```
   * editEntityElevation("bottom", "2"); // band starts at elevation 2, top unchanged
   * ```
   */
  function editEntityElevation(end: "bottom" | "top", raw: string): void {
    const doc = editingDoc;
    if (!doc) return;
    const eng = doc.engine as {
      /** The raw stored elevation band; absent/`null` means "every elevation". */
      elevation?: ElevationBand | null;
    };
    const old = eng.elevation ?? null;
    const next = editElevationBand(old, end, raw);
    if (next === undefined) return;
    editSelected("/engine/elevation", old, next);
  }

  /** The largest proximity distance, in cells, the server accepts (`MAX_PROXIMITY_CELLS`). */
  const MAX_PROXIMITY_CELLS = 1000;

  /** The senses a wall's proximity tier can open, in the order the editor lists them. */
  const PROXIMITY_SENSES: readonly WallSense[] = ["sight", "light", "sound"];

  /** Write the edited wall's whole `/engine/proximity` value; `old` is the raw stored value
   * (`null` when the wall has no tier). The server validates the tier as one unit, so every edit
   * replaces the object.
   * @param next The tier to store, or `null` to remove it.
   * @example
   * ```
   * editProximity({ within: 2, senses: ["sight"], soundGain: null });
   * ```
   */
  function editProximity(next: WallProximity | null): void {
    const doc = editingDoc;
    if (!doc) return;
    editSelected("/engine/proximity", (doc.engine as WallEngine).proximity ?? null, next);
  }

  /** Write the proximity distance from the input's raw string. A value that is not a finite
   * number in `(0, MAX_PROXIMITY_CELLS]` is ignored: the server rejects the whole update for one.
   * @param tier The wall's stored tier.
   * @param raw The distance input's raw string value, in cells.
   * @example
   * ```
   * editProximityWithin({ within: 1, senses: ["sight"], soundGain: null }, "3");
   * ```
   */
  function editProximityWithin(tier: WallProximity, raw: string): void {
    const within = Number(raw);
    if (raw.trim() === "" || !Number.isFinite(within) || within <= 0 || within > MAX_PROXIMITY_CELLS) return;
    if (within === tier.within) return;
    editProximity({ ...tier, within });
  }

  /** Add or remove one sense from the stored tier. Removing `sound` also clears the gain, which
   * the server accepts only alongside the sound sense.
   * @param tier The wall's stored tier.
   * @param sense The sense whose checkbox changed.
   * @param on Whether the sense is now checked.
   * @example
   * ```
   * editProximitySense({ within: 1, senses: ["sight"], soundGain: null }, "light", true);
   * ```
   */
  function editProximitySense(tier: WallProximity, sense: WallSense, on: boolean): void {
    const senses = PROXIMITY_SENSES.filter((s) => (s === sense ? on : tier.senses.includes(s)));
    editProximity({ ...tier, senses, soundGain: senses.includes("sound") ? (tier.soundGain ?? null) : null });
  }

  /** Whether an Escape keystroke is aimed at the canvas selection. Only a canvas-context Escape
   * backs out of the editor: a keydown originating inside a dialog (a floating panel closing on
   * the same keystroke) or a focused form control belongs to that surface.
   * @param event The Escape keydown event.
   * @returns True when the event's origin is neither inside a dialog nor a form control.
   * @example
   * ```
   * isCanvasEscape(new KeyboardEvent("keydown", { key: "Escape" })); // true
   * ```
   */
  function isCanvasEscape(event: KeyboardEvent): boolean {
    const origin = event.target;
    return !(
      origin instanceof Element &&
      (origin.closest('[role="dialog"]') !== null ||
        origin instanceof HTMLInputElement ||
        origin instanceof HTMLSelectElement ||
        origin instanceof HTMLTextAreaElement)
    );
  }

  // Escape backs out of any open scene-entity editor (clears the editing selection) through the
  // shared coordinator, registered only while an editor is open. Tool deactivation itself stays
  // with the rail's re-select-clears rule (`controller.toggle`).
  $effect(() => {
    if (!controller.editingEntity) return;
    return escapeCoordinator.register(
      ESCAPE_PRIORITY.TOOL,
      () => {
        controller.editingEntity = null;
      },
      isCanvasEscape,
    );
  });

  const triggerEvents: TriggerEvent[] = ["enter", "arrest", "exit", "turn_start", "turn_end", "round_start", "round_end"];
  /** The `TriggerEffect` discriminant vocabulary, mirroring the server's serde `type` tag. */
  type TriggerEffectType = RegionTrigger["effect"]["type"];
  const triggerEffectTypes: TriggerEffectType[] = ["condition_add", "condition_remove", "resource_delta", "chat_notice", "teleport", "scrolling_text"];
  const noticeAudiences: NoticeAudience[] = ["public", "gm_only", "owner"];

  /** Append a blank trigger row (a `condition_add` on `enter`) to the region tool's authored
   * list, persisted onto the next region the tool creates.
   * @example
   * ```
   * addRegionTrigger();
   * ```
   */
  function addRegionTrigger(): void {
    controller.regionTriggers.push({ on: "enter", effect: { type: "condition_add", condition: "" } });
  }

  /** Append a blank per-tag cost row (empty tag, cost 1) to the region tool's authored
   * `regionCostByTag` list, persisted onto the next region the tool creates.
   * @example
   * ```
   * addRegionCostByTagRow();
   * ```
   */
  function addRegionCostByTagRow(): void {
    controller.regionCostByTag.push({ tag: "", cost: 1 });
  }

  /** Arm the compound-region gesture: the region tool's drags then STAGE parts into the
   * `regionParts` list instead of persisting a single-shape region. A second click with
   * nothing staged yet cancels the gesture (the only escape hatch before the first drag —
   * an armed-but-empty list commits nothing either way).
   * @example
   * ```
   * toggleRegionCompound();
   * ```
   */
  function toggleRegionCompound(): void {
    if (controller.regionCompound && controller.regionParts.length === 0) {
      controller.discardRegionParts();
    } else {
      controller.regionCompound = true;
    }
  }

  /** Re-seat a row's effect when its type select changes. Each effect kind carries disjoint
   * fields, so a kind switch starts that row's payload fresh rather than carrying stale keys.
   * @param trig The trigger row being edited.
   * @param type The newly selected `TriggerEffect` discriminant.
   * @example
   * ```
   * declare const trig: RegionTrigger;
   * setRegionTriggerEffectType(trig, "chat_notice");
   * ```
   */
  function setRegionTriggerEffectType(trig: RegionTrigger, type: string): void {
    if (trig.effect.type === type) return;
    switch (type) {
      case "condition_remove": trig.effect = { type: "condition_remove", condition: "" }; break;
      case "resource_delta": trig.effect = { type: "resource_delta", resource: "", amount: 0 }; break;
      case "chat_notice": trig.effect = { type: "chat_notice", text: "", audience: "gm_only" }; break;
      case "teleport": trig.effect = { type: "teleport", target: { scene: null, x: 0, y: 0, elevation: null, vfx: null } }; break;
      case "scrolling_text": trig.effect = { type: "scrolling_text", text: "" }; break;
      default: trig.effect = { type: "condition_add", condition: "" };
    }
  }

  /** Parse an amount field into a `Formula`: a finite numeric literal stays a number, anything
   * else is kept as formula source text (the server parse-checks it at ingress).
   * @param trig The trigger row being edited (a no-op unless it is a `resource_delta`).
   * @param raw The raw text the amount input reports.
   * @example
   * ```
   * declare const trig: RegionTrigger;
   * setRegionTriggerAmount(trig, "1d6");
   * ```
   */
  function setRegionTriggerAmount(trig: RegionTrigger, raw: string): void {
    if (trig.effect.type !== "resource_delta") return;
    const n = Number(raw);
    trig.effect.amount = raw.trim() !== "" && Number.isFinite(n) ? n : raw;
  }
</script>


<div class="tool-rail" class:compact role="toolbar" aria-label={t("tools.title")}>
  {#each visibleTools as tool (tool.id)}
    <button
      type="button"
      class="tool"
      class:active={controller.active === tool.id}
      aria-pressed={controller.active === tool.id}
      data-testid="tool-{tool.id}"
      data-moves-frozen={movesFrozen && MOVE_TOOLS.includes(tool.id) ? "true" : undefined}
      title={movesFrozen && MOVE_TOOLS.includes(tool.id) ? t("tools.paused") : tool.label}
      onclick={() => controller.toggle(tool.id)}
    >
      {tool.label}
    </button>
  {/each}

  {#each sceneTools as tool (tool.id)}
    <button
      type="button"
      class="tool"
      class:active={controller.activeContributedId === tool.id}
      aria-pressed={controller.activeContributedId === tool.id}
      data-testid="scene-tool-{tool.id}"
      title={t(tool.labelKey)}
      onclick={() => controller.toggleContributed(tool)}
    >
      {t(tool.labelKey)}
    </button>
  {/each}

  <!-- Snap is a scene-document write (`/engine/snapToGrid`), i.e. authoring: GM-only. -->
  {#if isGm && activeScene}
    <button
      type="button"
      class="tool"
      aria-pressed={snapToGrid}
      data-testid="snap-toggle"
      title={t("tools.snap")}
      onclick={toggleSnap}
    >
      {t("tools.snap")}
    </button>
  {/if}

  <!-- Snap MODE (center, vertex or edge) steers only this client's own tool clicks, so it is
       client-local session state (`ctx.snapModePreference`), not a scene-document write;
       GM-only like the snap toggle beside it. -->
  {#if isGm && activeScene}
    <button
      type="button"
      class="tool"
      aria-pressed={ctx.snapModePreference.mode !== "center"}
      data-testid="snap-mode-toggle"
      title={t("tools.snapMode")}
      onclick={toggleSnapMode}
    >
      {snapModeLabel(ctx.snapModePreference.mode)}
    </button>
  {/if}

  <!-- The shared live ruler's toggle: client-local, per-user, OFF by default
       (`ctx.shareRuler`). Shown beside the measure tool's own controls, where the measure tool is
       active; never a document write. -->
  {#if controller.active === "measure"}
    <button
      type="button"
      class="tool"
      aria-pressed={ctx.shareRuler.enabled}
      aria-label={t("tools.shareRuler")}
      data-testid="share-ruler-toggle"
      title={t("tools.shareRuler")}
      onclick={() => ctx.shareRuler.set(!ctx.shareRuler.enabled)}
    >
      {t("tools.shareRuler")}
    </button>
  {/if}

  <!-- Camera shortcut for every role: centers on the average position of the viewer's own
       tokens on the viewed scene; disabled while the viewer owns none there. -->
  <button
    type="button"
    class="tool"
    disabled={ownedTokens.length === 0}
    aria-label={t("tools.panToOwnedTokens")}
    data-testid="pan-to-owned"
    title={t("tools.panToOwnedTokens")}
    onclick={panToOwnedTokens}
  >
    {t("tools.panToOwnedTokens")}
  </button>

  {#if canActAsSelected}
    <button
      type="button"
      class="tool"
      data-testid="speak-as-token"
      title={t("tools.speakAsToken")}
      onclick={speakAsSelectedToken}
    >
      {t("tools.speakAsToken")}
    </button>
  {/if}

  <!-- The emote palette is always rendered but disabled without a selection the user
       effectively owns (client-advisory; the server re-authorizes the relay). -->
  <div class="controls emote-palette" role="group" aria-label={t("tools.emote")}>
    {#each emoteChoices as emoji, i (emoji)}
      <button
        type="button"
        class="tool emote"
        disabled={!canActAsSelected}
        data-testid="emote-{i}"
        title={emoji}
        onclick={() => sendEmoteAsSelected(emoji)}
      >
        {emoji}
      </button>
    {/each}
    <input
      type="text"
      data-testid="emote-input"
      aria-label={t("tools.emoteCustom")}
      placeholder={t("tools.emoteCustom")}
      disabled={!canActAsSelected}
      bind:value={emoteDraft}
    />
    <button
      type="button"
      class="tool"
      data-testid="emote-send"
      disabled={!canActAsSelected || !emoteDraftSendable}
      title={t("tools.emoteSend")}
      onclick={sendEmoteDraft}
    >
      {t("tools.emoteSend")}
    </button>
  </div>

  <!-- Every mode control below belongs to a gmOnly tool. Gated on `isGm` as well as the
       active tool so the branch cannot render even if an authoring tool somehow became
       active for a non-GM. -->
  {#if isGm}
    {#if controller.active === "place"}
      <AssetPicker {controller} />
    {:else if controller.active === "draw"}
      <div class="controls">
        <select data-testid="draw-mode" aria-label={t("tools.shape")} bind:value={controller.drawMode}>
          {#each drawModes as m (m)}<option value={m}>{m}</option>{/each}
        </select>
        <input type="color" data-testid="draw-color" aria-label={t("tools.color")} bind:value={controller.strokeColor} />
      </div>
    {:else if controller.active === "template"}
      <div class="controls">
        <select data-testid="template-mode" aria-label={t("tools.shape")} bind:value={controller.templateMode}>
          {#each templateModes as m (m)}<option value={m}>{m}</option>{/each}
        </select>
        <input type="color" data-testid="template-color" aria-label={t("tools.color")} bind:value={controller.templateColor} />
        {#if controller.templateMode === "cone"}
          <!-- Cone full aperture, degrees; empty persists an absent `shape.angle` (the render
               layer's own DEFAULT_CONE_ANGLE_DEGREES). `min`/`max` bound the authorable range to
               `TemplateEngine::validate`'s own EXCLUSIVE `(0.0, 180.0)` — `conePoints`' 3-point
               wedge is collinear at 180° and opens away from `direction` past it. These native
               attributes are advisory only (they neither clamp a typed value nor block `change`);
               `isValidTemplateAngle` in the controller is the real gate at dispatch time. -->
          <input
            type="number"
            step="0.1"
            min="0.1"
            max="179.9"
            data-testid="template-angle"
            aria-label={t("tools.templateAngle")}
            value={controller.templateAngle ?? ""}
            onchange={(e) => {
              const parsed = parseElevation(e.currentTarget.value);
              if (parsed !== undefined) controller.templateAngle = parsed;
            }}
          />
        {/if}
        {#if controller.templateMode === "line" || controller.templateMode === "rect"}
          <!-- A line's thickness, or a rect's second dimension alongside `size`; empty persists
               an absent `shape.width` (zero-width line / forced-square rect). -->
          <input
            type="number"
            step="1"
            min="0"
            data-testid="template-width"
            aria-label={t("tools.templateWidth")}
            value={controller.templateWidth ?? ""}
            onchange={(e) => {
              const parsed = parseElevation(e.currentTarget.value);
              if (parsed !== undefined) controller.templateWidth = parsed;
            }}
          />
        {/if}
      </div>
    {:else if controller.active === "region"}
      <div class="controls">
        <select data-testid="region-shape" aria-label={t("tools.shape")} bind:value={controller.regionShapeMode}>
          {#each regionShapeModes as m (m)}<option value={m}>{m}</option>{/each}
        </select>
        <select data-testid="region-behavior" aria-label={t("tools.behavior")} bind:value={controller.regionBehavior}>
          {#each regionBehaviors as b (b)}<option value={b}>{b}</option>{/each}
        </select>
        <input type="number" data-testid="region-cost" aria-label={t("tools.cost")} min="1" step="0.5" bind:value={controller.regionCost} disabled={controller.regionBehavior !== "terrain"} />
        {#if controller.regionBehavior === "terrain"}
          <!-- Per-movement-tag cost overrides: the flat `regionCost` above stays the fallback
               for a mover whose tags match none of these rows. -->
          {#each controller.regionCostByTag as row, i (i)}
            <div class="trigger-row" data-testid="region-cost-by-tag-row">
              <input
                data-testid="region-cost-by-tag-tag"
                aria-label={t("tools.regionCostByTag")}
                bind:value={row.tag}
              />
              <input
                type="number"
                data-testid="region-cost-by-tag-cost"
                aria-label={t("tools.cost")}
                min="1"
                step="0.5"
                bind:value={row.cost}
              />
              <button
                type="button"
                data-testid="region-cost-by-tag-remove"
                aria-label={t("tools.removeTagCost")}
                title={t("tools.removeTagCost")}
                onclick={() => controller.regionCostByTag.splice(i, 1)}
              >×</button>
            </div>
          {/each}
          <button type="button" data-testid="region-cost-by-tag-add" onclick={addRegionCostByTagRow}>{t("tools.addTagCost")}</button>
        {/if}
        {#if controller.regionBehavior === "ambient_light"}
          <select data-testid="region-ambient-mode" aria-label={t("tools.regionAmbientLight")} bind:value={controller.regionAmbientMode}>
            {#each ambientLightModes as m (m)}<option value={m}>{m === "darken" ? t("tools.ambientDarken") : m === "brighten" ? t("tools.ambientBrighten") : t("tools.ambientOverride")}</option>{/each}
          </select>
          <input type="number" data-testid="region-ambient-level" aria-label={t("tools.ambientLevel")} min="0" max={controller.regionAmbientMode === "override" ? 1 : undefined} step="0.05" bind:value={controller.regionAmbientLevel} />
        {/if}
        <label>
          <input type="checkbox" data-testid="region-occludes-sight" bind:checked={controller.regionOccludesSight} />
          {t("tools.regionOccludesSight")}
        </label>
        <label>
          <input type="checkbox" data-testid="region-occludes-light" bind:checked={controller.regionOccludesLight} />
          {t("tools.regionOccludesLight")}
        </label>
        <label>
          <input type="checkbox" data-testid="region-occludes-sound" bind:checked={controller.regionOccludesSound} />
          {t("tools.regionOccludesSound")}
        </label>
        <label>
          <input type="checkbox" data-testid="region-secret" bind:checked={controller.regionSecret} />
          {t("tools.secret")}
        </label>
        <button type="button" data-testid="region-part-add" onclick={toggleRegionCompound}>{t("tools.regionPartAdd")}</button>
        {#if controller.regionCompound || controller.regionParts.length > 0}
          <!-- The compound part-list gesture: each row is one staged drag (additive unless its
               hole checkbox is set); nothing exists as a document until commit folds the
               whole list into ONE compound region. -->
          {#each controller.regionParts as part, i (i)}
            <div class="trigger-row">
              <span>{part.shape.kind}</span>
              <label>
                <input type="checkbox" data-testid="region-part-hole" bind:checked={part.hole} />
                {t("tools.regionPartHole")}
              </label>
              <button type="button" data-testid="region-part-remove" aria-label={t("tools.regionPartRemove")} title={t("tools.regionPartRemove")} onclick={() => controller.removeRegionPartAt(i)}>×</button>
            </div>
          {/each}
          <button
            type="button"
            data-testid="region-create-commit"
            disabled={controller.regionParts.length === 0}
            onclick={() => controller.commitRegionParts()}
          >{t("tools.regionCreateCommit")}</button>
        {/if}
        {#each controller.regionTriggers as trig, i (i)}
          <div class="trigger-row">
            <select data-testid="region-trigger-on" aria-label={t("tools.triggerOn")} bind:value={trig.on}>
              {#each triggerEvents as ev (ev)}<option value={ev}>{ev}</option>{/each}
            </select>
            <select
              data-testid="region-trigger-effect"
              aria-label={t("tools.triggerEffect")}
              value={trig.effect.type}
              onchange={(e) => setRegionTriggerEffectType(trig, e.currentTarget.value)}
            >
              {#each triggerEffectTypes as et (et)}<option value={et}>{et}</option>{/each}
            </select>
            {#if trig.effect.type === "condition_add" || trig.effect.type === "condition_remove"}
              <input data-testid="region-trigger-condition" aria-label={t("tools.triggerCondition")} bind:value={trig.effect.condition} />
            {:else if trig.effect.type === "resource_delta"}
              <input data-testid="region-trigger-resource" aria-label={t("tools.triggerResource")} bind:value={trig.effect.resource} />
              <input
                data-testid="region-trigger-amount"
                aria-label={t("tools.triggerAmount")}
                value={String(trig.effect.amount)}
                onchange={(e) => setRegionTriggerAmount(trig, e.currentTarget.value)}
              />
            {:else if trig.effect.type === "chat_notice"}
              <input data-testid="region-trigger-text" aria-label={t("tools.triggerText")} bind:value={trig.effect.text} />
              <select data-testid="region-trigger-audience" aria-label={t("tools.triggerAudience")} bind:value={trig.effect.audience}>
                {#each noticeAudiences as a (a)}<option value={a}>{a}</option>{/each}
              </select>
            {:else if trig.effect.type === "teleport"}
              <RegionTriggerTeleportEditor bind:target={trig.effect.target} row={i} {controller} />
            {:else if trig.effect.type === "scrolling_text"}
              <input
                data-testid="region-trigger-scrolling-text"
                aria-label={t("tools.triggerScrollingText")}
                title={t("tools.triggerScrollingTextHint", { max: MAX_OVERLAY_DISPLAY_CHARS })}
                bind:value={trig.effect.text}
              />
            {/if}
            <button type="button" data-testid="region-trigger-remove" title={t("tools.removeTrigger")} onclick={() => controller.regionTriggers.splice(i, 1)}>×</button>
          </div>
        {/each}
        <button type="button" data-testid="region-trigger-add" onclick={addRegionTrigger}>{t("tools.addTrigger")}</button>
      </div>
    {/if}

    <!-- Every entity editor below keys off the shared editing selection, not the active tool:
         both the select tool and the light tool can open one. The light editor reuses
         `LightEmissionEditor` (the same field set every emission surface edits) and commits the
         WHOLE `/engine/emission` payload with the raw stored emission as `old` — the same
         whole-payload convention the actor/sheet/token surfaces use. The wall/region/drawing/
         template editors share `editElevationBand` for their `/engine/elevation` band inputs. -->
    {#if editingDoc && controller.editingEntity?.kind === "light"}
      {@const eng = editingDoc.engine as LightEngine}
      <div class="controls" data-testid="light-editor">
        <LightEmissionEditor
          value={eng.emission}
          onCommit={(next) => editSelected("/engine/emission", eng.emission, next)}
        />
        <!-- The light's own elevation (absent = ground): a raised light shines over any wall
             whose band ends below it (`wall_occludes`'s source-elevation band test). -->
        <label>
          {t("tools.lightElevation")}
          <input
            type="number"
            step="1"
            data-testid="light-elevation"
            value={eng.elevation ?? 0}
            onchange={(e) => editLightElevation(e.currentTarget.value)}
          />
        </label>
        <button type="button" class="tool" data-testid="light-delete" onclick={deleteSelected}>{t("tools.delete")}</button>
      </div>
    {:else if editingDoc && controller.editingEntity?.kind === "wall"}
      <!-- `?? null` on each flag read: `WallEngine`'s flags are optional on the wire, so an
           absent key reads `undefined` at runtime — and `undefined` is dropped by JSON
           serialization, which would corrupt the OCC pre-image (the raw-`old` convention). -->
      {@const eng = editingDoc.engine as WallEngine}
      <div class="controls" data-testid="wall-editor">
        <label>
          <input
            type="checkbox"
            data-testid="wall-blocks-sight"
            checked={eng.blocksSight}
            onchange={(e) => editSelected("/engine/blocksSight", eng.blocksSight ?? null, e.currentTarget.checked)}
          />
          {t("tools.blocksSight")}
        </label>
        <label>
          <input
            type="checkbox"
            data-testid="wall-blocks-move"
            checked={eng.blocksMove}
            onchange={(e) => editSelected("/engine/blocksMove", eng.blocksMove ?? null, e.currentTarget.checked)}
          />
          {t("tools.blocksMove")}
        </label>
        <label>
          <input
            type="checkbox"
            data-testid="wall-blocks-light"
            checked={eng.blocksLight}
            onchange={(e) => editSelected("/engine/blocksLight", eng.blocksLight ?? null, e.currentTarget.checked)}
          />
          {t("tools.blocksLight")}
        </label>
        <!-- `blocksSound` absent = inherits the wall's door-resolved blocksSight server-side
             (`WallEngine::effective_flags`); the checkbox authors an explicit override only. -->
        <label>
          <input
            type="checkbox"
            data-testid="wall-blocks-sound"
            checked={eng.blocksSound}
            onchange={(e) => editSelected("/engine/blocksSound", eng.blocksSound ?? null, e.currentTarget.checked)}
          />
          {t("tools.blocksSound")}
        </label>
        <!-- One-way walls: `direction` carries the sign naming the SOLID side of the segment
             (`WallDirection.blockingSide`, ±1 — the cross-product convention is documented on
             that type). The checkbox writes `{ blockingSide: 1 }` / removes the field; the flip
             button negates the stored sign. Sight and movement only — light/sound stay
             symmetric. -->
        <label>
          <input
            type="checkbox"
            data-testid="wall-one-way"
            checked={eng.direction != null}
            onchange={(e) =>
              editSelected("/engine/direction", eng.direction ?? null, e.currentTarget.checked ? { blockingSide: 1 } : null)}
          />
          {t("tools.wallOneWay")}
        </label>
        {#if eng.direction}
          <button
            type="button"
            class="tool"
            data-testid="wall-flip-side"
            onclick={() =>
              editSelected("/engine/direction", eng.direction ?? null, { blockingSide: -(eng.direction?.blockingSide ?? 1) })}
          >
            {t("tools.wallFlipSide")}
          </button>
        {/if}
        <!-- `sightTier: "limited"`: a sight ray may cross this one wall and stops at the next
             crossing ("see through one such wall, not two"). Sight only. -->
        <label>
          <input
            type="checkbox"
            data-testid="wall-limited"
            checked={eng.sightTier === "limited"}
            onchange={(e) =>
              editSelected("/engine/sightTier", eng.sightTier ?? null, e.currentTarget.checked ? "limited" : null)}
          />
          {t("tools.wallLimited")}
        </label>
        <!-- `proximity`: the wall is absent for a sight ray, light or sound whose origin lies
             within `within` cells of it. The last remaining sense cannot be unchecked (a tier
             names at least one sense); the toggle removes the tier. The gain slider exists only
             with the sound sense (`soundGain` is rejected without it). -->
        <label>
          <input
            type="checkbox"
            data-testid="wall-proximity"
            checked={eng.proximity != null}
            onchange={(e) =>
              editProximity(e.currentTarget.checked ? { within: 1, senses: ["sight"], soundGain: null } : null)}
          />
          {t("tools.wallProximity")}
        </label>
        {#if eng.proximity}
          {@const tier = eng.proximity}
          <label>
            {t("tools.wallProximityWithin")}
            <input
              type="number"
              min="0"
              max={MAX_PROXIMITY_CELLS}
              step="0.5"
              data-testid="wall-proximity-within"
              value={tier.within}
              onchange={(e) => editProximityWithin(tier, e.currentTarget.value)}
            />
          </label>
          {#each PROXIMITY_SENSES as sense (sense)}
            <label>
              <input
                type="checkbox"
                data-testid={`wall-proximity-${sense}`}
                checked={tier.senses.includes(sense)}
                disabled={tier.senses.length === 1 && tier.senses.includes(sense)}
                onchange={(e) => editProximitySense(tier, sense, e.currentTarget.checked)}
              />
              {t(`tools.wallProximity.${sense}`)}
            </label>
          {/each}
          {#if tier.senses.includes("sound")}
            <label>
              {t("tools.wallProximityGain")}
              <input
                type="range"
                min="0"
                max="1"
                step="0.05"
                data-testid="wall-proximity-gain"
                value={tier.soundGain ?? 1}
                onchange={(e) => editProximity({ ...tier, soundGain: Number(e.currentTarget.value) })}
              />
            </label>
          {/if}
        {/if}
        <!-- The door state: `none` (an ordinary wall — writes `null`, removing the door) or
             `closed`/`open`/`locked`. This select is the GM's authoring surface (and the ONLY
             way onto or off `locked`); the player-facing flip is the select tool's
             click-to-toggle (`toggleDoorAt`), which never routes through this editor. -->
        <label>
          {t("tools.doorToggle")}
          <select
            data-testid="wall-door"
            aria-label={t("tools.doorToggle")}
            value={eng.door ?? "none"}
            onchange={(e) =>
              editSelected("/engine/door", eng.door ?? null, e.currentTarget.value === "none" ? null : e.currentTarget.value)}
          >
            <option value="none">—</option>
            <option value="closed">closed</option>
            <option value="open">open</option>
            <option value="locked">locked</option>
          </select>
        </label>
        <!-- The elevation band the wall's sight/light occlusion applies to; both ends empty =
             unbounded (stored as an absent band, which occludes every elevation). -->
        <label>
          {t("tools.wallElevationBottom")}
          <input
            type="number"
            step="1"
            data-testid="wall-elevation-bottom"
            value={eng.elevation?.bottom ?? ""}
            onchange={(e) => editEntityElevation("bottom", e.currentTarget.value)}
          />
        </label>
        <label>
          {t("tools.wallElevationTop")}
          <input
            type="number"
            step="1"
            data-testid="wall-elevation-top"
            value={eng.elevation?.top ?? ""}
            onchange={(e) => editEntityElevation("top", e.currentTarget.value)}
          />
        </label>
        <button type="button" class="tool" data-testid="wall-delete" onclick={deleteSelected}>{t("tools.delete")}</button>
      </div>
    {:else if editingDoc && controller.editingEntity?.kind === "region"}
      <!-- The elevation band a region's geometry occupies; both ends empty = unbounded
           (stored as an absent band, occupying every level — `band_contains`). -->
      {@const eng = editingDoc.engine as RegionEngine}
      <div class="controls" data-testid="region-editor">
        <label>
          {t("tools.regionElevationBottom")}
          <input
            type="number"
            step="1"
            data-testid="region-elevation-bottom"
            value={eng.elevation?.bottom ?? ""}
            onchange={(e) => editEntityElevation("bottom", e.currentTarget.value)}
          />
        </label>
        <label>
          {t("tools.regionElevationTop")}
          <input
            type="number"
            step="1"
            data-testid="region-elevation-top"
            value={eng.elevation?.top ?? ""}
            onchange={(e) => editEntityElevation("top", e.currentTarget.value)}
          />
        </label>
        {#each OCCLUDE_SENSES as sense (sense)}
          <label>
            <input
              type="checkbox"
              data-testid={`region-editor-occludes-${sense}`}
              checked={eng.occludes?.[sense] ?? false}
              onchange={(e) => editRegionOcclusion(eng, sense, e.currentTarget.checked)}
            />
            {t(OCCLUDE_LABEL[sense])}
          </label>
        {/each}
        <!-- Attach the region to a token on this scene. Picking a token or ticking a flag
             re-expresses the stored shape so the region does not move on the canvas. A list
             (not a pointer gesture) so it works on touch. -->
        <label>
          {t("tools.regionFollowToken")}
          <select
            data-testid="region-follow-token"
            value={eng.anchor?.token ?? ""}
            onchange={(e) => editRegionAnchor(eng, e.currentTarget.value, {})}
          >
            <option value="">{t("tools.regionFollowNone")}</option>
            {#each regionCarrierOptions(editingDoc) as opt (opt.id)}
              <option value={opt.id}>{opt.label}</option>
            {/each}
          </select>
        </label>
        {#if eng.anchor}
          <label>
            <input
              type="checkbox"
              data-testid="region-follow-include-carrier"
              checked={eng.anchor.include_carrier}
              onchange={(e) => editRegionAnchor(eng, undefined, { include_carrier: e.currentTarget.checked })}
            />
            {t("tools.regionFollowIncludeCarrier")}
          </label>
          <label>
            <input
              type="checkbox"
              data-testid="region-follow-rotation"
              checked={eng.anchor.follow_rotation}
              onchange={(e) => editRegionAnchor(eng, undefined, { follow_rotation: e.currentTarget.checked })}
            />
            {t("tools.regionFollowRotation")}
          </label>
        {/if}
        <button type="button" class="tool" data-testid="region-delete" onclick={deleteSelected}>{t("tools.delete")}</button>
      </div>
    {:else if editingDoc && controller.editingEntity?.kind === "drawing"}
      <!-- The elevation band a drawing's geometry occupies; both ends empty = unbounded. -->
      {@const eng = editingDoc.engine as DrawingEngine}
      <div class="controls" data-testid="drawing-editor">
        <label>
          {t("tools.drawingElevationBottom")}
          <input
            type="number"
            step="1"
            data-testid="drawing-elevation-bottom"
            value={eng.elevation?.bottom ?? ""}
            onchange={(e) => editEntityElevation("bottom", e.currentTarget.value)}
          />
        </label>
        <label>
          {t("tools.drawingElevationTop")}
          <input
            type="number"
            step="1"
            data-testid="drawing-elevation-top"
            value={eng.elevation?.top ?? ""}
            onchange={(e) => editEntityElevation("top", e.currentTarget.value)}
          />
        </label>
        <button type="button" class="tool" data-testid="drawing-delete" onclick={deleteSelected}>{t("tools.delete")}</button>
      </div>
    {:else if editingDoc && controller.editingEntity?.kind === "template"}
      <!-- The elevation band a template's geometry occupies; both ends empty = unbounded. -->
      {@const eng = editingDoc.engine as TemplateEngine}
      <div class="controls" data-testid="template-editor">
        <label>
          {t("tools.templateElevationBottom")}
          <input
            type="number"
            step="1"
            data-testid="template-elevation-bottom"
            value={eng.elevation?.bottom ?? ""}
            onchange={(e) => editEntityElevation("bottom", e.currentTarget.value)}
          />
        </label>
        <label>
          {t("tools.templateElevationTop")}
          <input
            type="number"
            step="1"
            data-testid="template-elevation-top"
            value={eng.elevation?.top ?? ""}
            onchange={(e) => editEntityElevation("top", e.currentTarget.value)}
          />
        </label>
        <!-- Conversion needs both create (region) and delete (template) rights; a shape with no
             region equivalent shows its reason as text, since a tooltip is unreachable on touch. -->
        {#if ctx.canCreate("region") && ctx.canDelete(editingDoc)}
          {@const conversion = templateToRegionShape(eng.shape)}
          <button
            type="button"
            class="tool"
            data-testid="template-make-region"
            disabled={!conversion.ok}
            onclick={() => controller.convertTemplateToRegion(editingDoc)}
          >{t("tools.templateMakeRegion")}</button>
          {#if !conversion.ok}
            <span data-testid="template-make-region-reason">{t(conversion.reason === "zero_width_line" ? "tools.templateMakeRegionZeroWidth" : "tools.templateMakeRegionUnknownShape")}</span>
          {/if}
        {/if}
        <button type="button" class="tool" data-testid="template-delete" onclick={deleteSelected}>{t("tools.delete")}</button>
      </div>
    {/if}
  {/if}
</div>

<style lang="scss">
  .tool-rail {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-1);
  }
  .tool {
    /* Touch target floor (WCAG 2.5.5 / iOS HIG); the rail's own column width
     * (`Layout.svelte`'s `.layout` grid-template-columns) derives from this same
     * token so a tool button never gets clipped by its own containing cell. */
    min-height: var(--input-height-coarse);
    min-width: var(--input-height-coarse);
    max-width: 100%;
    box-sizing: border-box;
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-1);
    background: var(--surface-raised);
    color: var(--text-primary);
    cursor: pointer;
  }
  .tool.active {
    background: var(--accent);
    color: var(--on-accent);
    border-color: var(--accent);
  }
  .tool:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .controls {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .controls select,
  .controls input {
    min-height: 32px;
    min-width: 0;
    max-width: 100%;
    box-sizing: border-box;

    @media (pointer: coarse) {
      min-height: var(--input-height-coarse);
    }
  }
  .trigger-row {
    display: flex;
    flex-direction: row;
    gap: var(--space-1);
    align-items: center;
  }
  .trigger-row input {
    min-width: 0;
    flex: 1;
  }

  /* Emote palette: a wrapping row of glyph buttons plus the free input. */
  .emote-palette {
    flex-direction: row;
    flex-wrap: wrap;
    align-items: center;
  }
  .tool.emote {
    font-size: 1.25rem;
    padding: var(--space-1);
  }
  .emote-palette input {
    min-width: 0;
    width: 8rem;
    max-width: 100%; /* the vertical rail is narrower than 8rem; fit it rather than overflow */
    box-sizing: border-box;
  }

  /* Compact bottom strip: lay tools out horizontally with overflow scroll
   * instead of a vertical column; the active-tool controls follow suit. */
  .tool-rail.compact {
    flex-direction: row;
    flex-wrap: nowrap;
    align-items: center;
    overflow-x: auto;
  }
  .tool-rail.compact .controls {
    flex-direction: row;
    align-items: center;
  }
</style>
