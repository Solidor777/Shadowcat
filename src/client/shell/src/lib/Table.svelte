<script lang="ts">
  import { setAppContext, Surface, PanelsBridge, SheetsController, SceneSelection } from "@shadowcat/ui-kit";
  import { t } from "@shadowcat/ui-kit";
  import { consoleLogger } from "@shadowcat/core";
  import { createSubscriber } from "svelte/reactivity";
  import { logout } from "./api";
  import { navigate } from "./route.svelte";
  import { getPanelLayout, setPanelLayout } from "./sessionState.svelte";
  import type { WorldSession } from "./worldSession.svelte";

  // `PanelHost` binds the real implementation into this bridge at its own
  // mount (later than `Table`'s own init); calls made before that bind are
  // a no-op, warned once via the injected logger.
  const panels = new PanelsBridge(consoleLogger());

  let { session, leaveWorld }: { session: WorldSession; leaveWorld: () => void } =
    $props();

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

  // Boot restore (§7): re-open every persisted sheet whose document resolves. Sheets are
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
    selfId: session.selfId,
    canEdit: (doc, path) => session.canEdit(doc, path),
    openDocument: (ref) => sheets.openDocument(ref),
    members: session.members,
    t,
    assets: session.assets,
    onAssetChanged: (cb) => session.onAssetChanged(cb),
    subscribeScene: (c, cb, opts) => session.subscribeScene(c, cb, opts),
    dispatchIntent: (ops) => session.dispatchIntent(ops),
    scene: session.sceneInteraction,
    actorSelection: session.actorSelection,
    tokenSelection: session.tokenSelection,
    get viewedSceneId() {
      return session.viewedSceneId;
    },
    setGmViewedScene: (id) => session.setGmViewedScene(id),
    searchDocuments: (query, opts, onUpdate) => session.searchDocuments(query, opts, onUpdate),
    sceneSelection,
    sendPing: (x, y) => session.sendPing(x, y),
    pathfind: (s, st, wp, fr) => session.pathfind(s, st, wp, fr),
    moveRequest: (s, tid, p) => session.moveRequest(s, tid, p),
    onPing: (cb) => session.onPing(cb),
    chat: {
      send: (o) => session.sendChatMessage(o),
      edit: (id, c) => session.editChatMessage(id, c),
      delete: (id) => session.deleteChatMessage(id),
    },
    uiState: {
      getPanelLayout: () => getPanelLayout(session.world!),
      setPanelLayout: (blob) => setPanelLayout(session.world!, blob),
    },
    panels,
    leaveWorld,
    logout: async () => {
      await logout();
      navigate({ name: "login" });
    },
    // TODO: replace with the real TemplatesController wiring; all methods currently no-op.
    templates: {
      stampInstance: (s) => s,
      pull: () => {},
      push: () => {},
      revert: () => {},
      findInstances: () => [],
      syncState: () => "none",
      canPull: () => false,
      canPush: () => false,
    },
  });
</script>

<Surface contract="shadowcat.surface:root" />
