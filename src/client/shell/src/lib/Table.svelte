<script lang="ts">
  import { setAppContext, Surface, PanelsBridge, SheetsController, SceneSelection, SpeakAs, SpeakAsToken, TemplatesController, TemplateModalHost, NotificationHost, notifications, AssetPickController, type PickAssetOptions, type AppContext } from "@shadowcat/ui-kit";
  import { t } from "@shadowcat/ui-kit";
  import { consoleLogger } from "@shadowcat/core";
  import { createSubscriber } from "svelte/reactivity";
  import { logout } from "./api";
  import { navigate } from "./route.svelte";
  import { getPanelLayout, setPanelLayout, getChatRead, setChatRead, resetSessionState } from "./sessionState.svelte";
  import type { WorldSession } from "./worldSession.svelte";

  // `PanelHost` binds the real implementation into this bridge at its own
  // mount (later than `Table`'s own init); calls made before that bind are
  // a no-op, warned once via the injected logger.
  const panels = new PanelsBridge(consoleLogger());

  let {
    session,
    leaveWorld,
    serverRole,
  }: {
    /** The entered world's session, non-null by the time `App` mounts `Table` (it
     * renders `Table` only once role+world are set from Welcome). */
    session: WorldSession;
    /** Tears down `session` and returns the shell to the entry/worlds route. */
    leaveWorld: () => void;
    /** The caller's server tier — see `AppContext.serverRole` for the fail-closed
     * derivation and its cosmetic-only status. */
    serverRole: "admin" | "user";
  } = $props();

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

  // Template merge controller: stamp/pull/push/revert orchestration + the conflict modal.
  // `session` is fixed per Table, so capturing it once here is intended (see the identical
  // rationale on the `setAppContext` call below).
  // svelte-ignore state_referenced_locally
  const templates = new TemplatesController({
    store: session.store,
    documents: session.documents,
    dispatchIntent: (ops) => session.dispatchIntent(ops),
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

  // App renders <Table> only once role+world are set (Welcome received), so these
  // are non-null at init. setContext must run during init, not in markup; the
  // session/leaveWorld are fixed per Table, so capturing them once is intended.
  // svelte-ignore state_referenced_locally
  setAppContext({
    contributions: session.contributions,
    store: session.store,
    documents: session.documents,
    world: session.world!,
    role: session.role!,
    serverRole,
    selfId: session.selfId,
    canEdit: (doc, path) => session.canEdit(doc, path),
    openDocument: (ref) => sheets.openDocument(ref),
    notify: (message, level) => notifications.push(level ?? "warning", message),
    members: session.members,
    t,
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
    searchDocuments: (query, opts, onUpdate) => session.searchDocuments(query, opts, onUpdate),
    sceneSelection,
    speakAsToken,
    speakAs,
    sendPing: (x, y) => session.sendPing(x, y),
    sendEmote: (token, emote) => session.sendEmote(token, emote),
    pathfind: (s, st, wp, fr, tk) => session.pathfind(s, st, wp, fr, tk),
    moveRequest: (s, tid, p) => session.moveRequest(s, tid, p),
    onPing: (cb) => session.onPing(cb),
    onEmote: (cb) => session.onEmote(cb),
    onMoveOutcome: (cb) => session.onMoveOutcome(cb),
    chat: {
      send: (o) => session.sendChatMessage(o),
      edit: (id, c) => session.editChatMessage(id, c),
      delete: (id) => session.deleteChatMessage(id),
      recalc: (id, rollId, ops) => session.recalcRoll(id, rollId, ops),
    },
    uiState: {
      getPanelLayout: () => getPanelLayout(session.world!),
      setPanelLayout: (blob) => setPanelLayout(session.world!, blob),
      getChatRead: () => getChatRead(session.world!),
      setChatRead: (blob) => setChatRead(session.world!, blob),
    },
    panels,
    reconcileInstalledModules: () => session.reconcileInstalledModules(),
    leaveWorld,
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
