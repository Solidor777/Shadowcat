<script lang="ts">
  import { setAppContext, Surface, PanelsBridge, SheetsController, SceneSelection, SpeakAs, SpeakAsToken, SnapModePreference, StageView, TemplatesController, TemplateModalHost, NotificationHost, DialogHost, notifications, AssetPickController, performanceController, type PickAssetOptions, type AppContext } from "@shadowcat/ui-kit";
  import { t, plural, compare } from "@shadowcat/ui-kit";
  import type { TlsStatus } from "@shadowcat/types";
  import { consoleLogger } from "@shadowcat/core";
  import { createSubscriber } from "svelte/reactivity";
  import { untrack } from "svelte";
  import { logout } from "./api";
  import { navigate } from "./route.svelte";
  import { getPanelLayout, setPanelLayout, getChatRead, setChatRead, getChatHistory, pushChatHistory, resetSessionState } from "./sessionState.svelte";
  import type { WorldSession } from "./worldSession.svelte";
  import { createPresentedNoteHandler } from "./presentedNote";

  // `PanelHost` binds the real implementation into this bridge at its own
  // mount (later than `Table`'s own init); calls made before that bind are
  // a no-op, warned once via the injected logger.
  const panels = new PanelsBridge(consoleLogger());

  let {
    session: sessionProp,
    leaveWorld,
    serverRole,
    tls,
  }: {
    /** The entered world's session, non-null by the time `App` mounts `Table` (it
     * renders `Table` only once role+world are set from Welcome). */
    session: WorldSession;
    /** Tears down `session` and returns the shell to the entry/worlds route. */
    leaveWorld: () => void;
    /** The caller's server tier — see `AppContext.serverRole` for the fail-closed
     * derivation and its cosmetic-only status. */
    serverRole: "admin" | "user";
    /** The server's published TLS status, `null` when it could not be read; see
     * `AppContext.tls`. */
    tls: TlsStatus | null;
  } = $props();

  // One `session` for this Table's whole life: every controller and context entry below is built
  // from it once, at init. `App` renders `Table` inside `{#key session}`, so a different session
  // mounts a fresh Table instead of reaching this one as a changed prop; the untracked read states
  // that once-only capture. `serverRole`, `tls` and `leaveWorld` can change while mounted, so the
  // context reads them live (getters and a closure).
  const session = untrack(() => sessionProp);
  // Fails loudly if a caller swaps the session on a mounted Table instead of remounting it: the
  // controllers above were built from the first one and would keep serving it.
  $effect(() => {
    if (sessionProp !== session) {
      throw new Error("Table received a different session than it was mounted with; render it inside {#key session}");
    }
  });

  // Sheet panels: the controller registers `sheet:<docId>` contributions on demand; the
  // panel host mounts + floats them. Constructed before setAppContext so `openDocument` is
  // on the context from the first render.
  const sheets = new SheetsController({
    contributions: session.contributions,
    documents: session.documents,
    panels,
    logger: consoleLogger(),
  });

  // Scene "Configure" focus: the browser sets it, GameSettingsPanel reads it. Stable per Table,
  // like `panels`/`sheets`.
  const sceneSelection = new SceneSelection();

  // Asset pick-mode orchestration: `pickAsset` requests land here; the
  // asset-browser module's overlay contribution renders `pending` and settles
  // it. Stable per Table, like the selections above.
  const assetPick = new AssetPickController();

  // Speak-as-token pending selection: the scene-tools affordance sets it, the composer
  // consumes it on send. Stable per Table, like `sceneSelection`.
  const speakAsToken = new SpeakAsToken();

  // Sticky speak-as actor selection: every roll-producing surface (composer, chat-card
  // buttons) resolves the same session-level selection. Stable per Table, like `speakAsToken`.
  const speakAs = new SpeakAs();

  // Snap-mode preference (center vs vertex): the ToolRail toggle writes it, Stage pushes it
  // to the engine's snap chokepoint. Client-local session state — never a document write.
  // Stable per Table, like `speakAs`.
  const snapModePreference = new SnapModePreference();

  // Stage camera mirror: Stage writes the applied transform, stage overlays project scene
  // points through it. Stable per Table, like `snapModePreference`.
  const stageView = new StageView();

  // Template merge controller: stamp/pull/push/revert orchestration + the conflict modal.
  // `session` is fixed per Table (see its capture above).
  const templates = new TemplatesController({
    store: session.store,
    documents: session.documents,
    sendMergeIntent: (msg, opts) => session.mergeIntent(msg, opts),
    role: session.role!,
    selfId: session.selfId,
    canEdit: (doc, path) => session.canEdit(doc, path),
    logger: consoleLogger(),
    notify: (message, level) => notifications.push(level ?? "warning", message),
  });

  // Boot restore: re-open every persisted sheet whose document resolves. Sheets are
  // registered only when their doc is present, so this runs reactively — panels mount
  // during #onWelcome BEFORE the resync stream fills the store, so a one-shot scan would
  // find no resolvable docs. `createSubscriber` re-runs it as the store fills;
  // `restoreFromPersisted` is idempotent, so re-runs never duplicate.
  const subscribe = createSubscriber((update) => session.documents.subscribe(update));
  $effect(() => {
    subscribe();
    sheets.restoreFromPersisted(getPanelLayout(session.world!));
  });

  // The GM's "present this note" signal opens the note's sheet for every other client. The
  // returned unsubscribe is the effect's cleanup.
  const onPresentNote = createPresentedNoteHandler({
    documents: session.documents,
    selfId: session.selfId,
    openDocument: (ref) => sheets.openDocument(ref),
    notify: (message, level) => notifications.push(level ?? "warning", message),
    t,
  });
  $effect(() => session.onPresentNote(onPresentNote));

  // App renders <Table> only once role+world are set (Welcome received), so these
  // are non-null at init. setContext must run during init, not in markup; the
  // session is fixed per Table, so capturing it once is intended.
  setAppContext({
    contributions: session.contributions,
    store: session.store,
    documents: session.documents,
    combat: session.combat,
    world: session.world!,
    role: session.role!,
    // A getter, not a captured value: the caller's tier is a prop that follows the loaded account.
    get serverRole() {
      return serverRole;
    },
    // A getter, not a captured value: `tls` is a prop the shell refreshes after a failed first
    // read, and consumers must see the current one.
    get tls() {
      return tls;
    },
    selfId: session.selfId,
    canEdit: (doc, path) => session.canEdit(doc, path),
    canCreate: (docType) => session.canCreate(docType),
    canDelete: (doc) => session.canDelete(doc),
    openDocument: (ref) => sheets.openDocument(ref),
    notify: (message, level) => notifications.push(level ?? "warning", message),
    notifyProgress: (message, progress) => {
      const id = notifications.push("info", message, undefined, progress);
      return {
        id,
        update: (patch) => notifications.update(id, patch),
        dismiss: () => notifications.dismiss(id),
      };
    },
    members: session.members,
    presence: session.presence,
    t,
    plural,
    compare,
    assets: session.assets,
    assetPick,
    pickAsset: ((opts?: PickAssetOptions) =>
      assetPick
        .request(opts ?? {})
        .then((ids) => (opts?.multiple ? ids : (ids?.[0] ?? null)))) as AppContext["pickAsset"],
    onAssetChanged: (cb) => session.onAssetChanged(cb),
    subscribeScene: (c, cb, opts) => session.subscribeScene(c, cb, opts),
    dispatchIntent: (ops) => session.dispatchIntent(ops),
    scene: session.sceneInteraction,
    actorSelection: session.actorSelection,
    tokenSelection: session.tokenSelection,
    get viewedSceneId() {
      return session.viewedSceneId;
    },
    get footprints() {
      return session.footprints;
    },
    setGmViewedScene: (id) => session.setGmViewedScene(id),
    get viewedLevel() {
      return session.viewedLevel;
    },
    setViewedLevel: (id) => session.setViewedLevel(id),
    searchDocuments: (query, opts, onUpdate) => session.searchDocuments(query, opts, onUpdate),
    sceneSelection,
    speakAsToken,
    speakAs,
    snapModePreference,
    stageView,
    shareRuler: session.shareRuler,
    sendRulerUpdate: (scene, points, active) => session.sendRulerUpdate(scene, points, active),
    onRuler: (cb) => session.onRuler(cb),
    sendPing: (x, y) => session.sendPing(x, y),
    sendEmote: (token, emote) => session.sendEmote(token, emote),
    resetExploredFog: (scene, opts) => session.resetExploredFog(scene, opts),
    pathfind: (s, st, wp, fr, tk) => session.pathfind(s, st, wp, fr, tk),
    moveRequest: (s, tid, p, g) => session.moveRequest(s, tid, p, g),
    toggleDoor: (s, w, to) => session.toggleDoor(s, w, to),
    onPing: (cb) => session.onPing(cb),
    onEmote: (cb) => session.onEmote(cb),
    onScrollingText: (cb) => session.onScrollingText(cb),
    onMoveOutcome: (cb) => session.onMoveOutcome(cb),
    chat: {
      send: (o) => session.sendChatMessage(o),
      edit: (id, c) => session.editChatMessage(id, c),
      delete: (id) => session.deleteChatMessage(id),
      recalc: (id, rollId, ops) => session.recalcRoll(id, rollId, ops),
      reviseAudience: (id, audience) => session.reviseAudience(id, audience),
      drawTable: (opts) => session.drawTable(opts),
    },
    uiState: {
      getPanelLayout: () => getPanelLayout(session.world!),
      setPanelLayout: (blob) => setPanelLayout(session.world!, blob),
      getChatRead: () => getChatRead(session.world!),
      setChatRead: (blob) => setChatRead(session.world!, blob),
      getChatHistory: () => getChatHistory(session.world!),
      pushChatHistory: (text) => pushChatHistory(session.world!, text),
    },
    panels,
    dice3d: session.dice3d,
    audio: session.audio,
    performance: performanceController,
    vfx: { play: (req) => session.playVfx(req), onVfx: (cb) => session.onVfx(cb) },
    notes: { present: (noteId) => session.presentNote(noteId) },
    hooks: session.hooks,
    reconcileInstalledModules: () => session.reconcileInstalledModules(),
    moduleStatus: session.moduleStatus,
    activeModuleManifests: () => session.activeModuleManifests(),
    leaveWorld: () => leaveWorld(),
    logout: async () => {
      await logout();
      resetSessionState();
      navigate({ name: "login" });
    },
    templates: {
      stampInstance: (s, opts) => templates.stampInstance(s, opts),
      pull: (id) => templates.pull(id),
      push: (id) => templates.push(id),
      revert: (id) => templates.revert(id),
      findInstances: (id) => templates.findInstances(id),
      syncState: (id) => templates.syncState(id),
      canPull: (id) => templates.canPull(id),
      canPush: (id) => templates.canPush(id),
    },
  });
</script>

<Surface contract="shadowcat.surface:root" />
<TemplateModalHost controller={templates} />
<NotificationHost />
<DialogHost />
