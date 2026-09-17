import {
  WsClient,
  OptimisticClient,
  DocumentStore,
  ContributionRegistry,
  AssetResolver,
  type AssetChangedNotice,
  ModuleRegistry,
  HookBus,
  ServiceRegistry,
  MiddlewareChain,
  reconcileTopology,
  buildSceneDoc,
  resolveViewedScene,
  levelOf,
  type SceneEngine,
  type TokenEngine,
  consoleLogger,
  resolveCaps,
  ownerFloorApplies,
  canWritePath,
  canCreateDoc,
  parseFootprints,
  EMPTY_FOOTPRINTS,
  type FootprintLookup,
  parseAudibility,
  sceneAudibility,
  EMPTY_AUDIBILITY,
  type AudibilityPayload,
  CombatController,
  defineCombatHooks,
  CombatHookEmitter,
  commandTouchesCombat,
  deriveCombatHookEvents,
  COMBAT_SERVICE,
  COMBAT_HOOK_VERSION,
  parseCombats,
  EMPTY_COMBATS,
  type CombatApi,
  type Connect,
  type Logger,
  type Module,
  type WireWelcome,
  type WireOperation,
  type WireDocument,
  type WireCapabilityRequirement,
  type ChatSendOptions,
  type WireRecalcOp,
  type DrawTableOptions,
  type SceneFrame,
  type SceneSubscription,
  type PathResult,
  type MoveStream,
  type VfxNotice,
  type VfxPlayRequest,
  type SubscriptionHandle,
  type WireSearchHit,
  type ClientMsg,
  type WireMergeOutcome,
  type WsTimeoutOptions,
  loadModules,
  type ModuleManifest,
  listInstalledModules,
  getEnabledModules,
  listWorldMembers,
  AUDIO_STATE_DOC_TYPE,
  type AudioApi,
  type AudioChannelId,
  type AudioStateEngine,
} from "@shadowcat/core";
import type { WorldRole, InstalledModuleInfo, RejectReason } from "@shadowcat/types";
import { SceneInteractionBridge, ActorSelection, TokenSelection, i18n, performanceController } from "@shadowcat/ui-kit";
import { AudioEngine, DEFAULT_DUCK_DEPTH, setMediaElementFactory } from "@shadowcat/audio";
import { SvelteMap, createSubscriber } from "svelte/reactivity";
import { getWorldSnapshot } from "./api";
import { getViewedLevel, setViewedLevel as persistViewedLevel } from "./sessionState.svelte";
import { readAudioMirror, writeAudioMirror } from "./sessionState.svelte";

/** The WS connection lifecycle a `WorldSession` exposes as its reactive `state`. */
export type ConnState = "connecting" | "open" | "closed";

/** Options for {@link WorldSession.subscribeScene}. Shares its one field's shape with
 * `AppSubscribeSceneOptions` in `@shadowcat/ui-kit`'s `appContext` module by coincidence, not
 * by a package-boundary dependency — check that type too if this one's fields change. */
export interface SubscribeSceneOpts {
  /** GM-only see-as: resolve the channel as if for this user instead of self. */
  asUser?: string;
  /** The level to scope explored-fog accumulation/emission to (`"vision"` channel only;
   * ignored by every other channel) — forwarded to `WsClient.subscribeScene`'s own `level`. */
  level?: string;
}

/** One `subscribeScene` record, keyed by a locally-generated id in `WorldSession.#sceneSubs`
 * and (re-)established by `WorldSession.#establishScene`. */
interface SceneSubRecord {
  /** The SceneDerived channel name. */
  channel: string;
  /** Called with each new frame on that channel. */
  onUpdate: (f: SceneFrame) => void;
  /** GM-only see-as override, if this subscription used one. */
  asUser?: string;
  /** The level this subscription is scoped to (`"vision"` channel only). */
  level?: string;
  /** The currently-live WS handle, or `null` while (re-)establishing. */
  handle: SceneSubscription | null;
  /** Generation counter; bumped to invalidate a superseded establish attempt. */
  gen: number;
}

/** Construction options for `WorldSession`. */
export interface WorldSessionOpts {
  /** This client's own user id (ownership checks; see `WorldSession.selfId`). */
  selfId: string;
  /** Browser: webSocketConnect(wsUrl). Tests: a mock connect. */
  connect: Connect;
  /** First-party default modules, in activation order (the layout/core-ui module
   *  first so its region surfaces exist before panel modules activate). */
  modules: Module[];
  /** Diagnostics sink; defaults to the leveled console logger. */
  logger?: Logger;
  /** Terminal eviction (this world or this account was deleted). The WsClient
   *  has already stopped — the shell routes the user out of the world. */
  onEvicted?: () => void;
  /** Called after every rejected intent, with the server's reason and any player-presentable
   * detail text — the optimistic prediction has already been rolled back (`#optimistic.reject`)
   * by the time this fires. The shell surfaces it as a toast; a headless caller (tests) may
   * leave it unset. */
  onReject?: (reason: RejectReason, detail: string | null) => void;
  /** Called when an `audio_transport` op this connection sent was refused, with the server's
   * player-presentable reason (fire-and-forget frame — there is no correlated reply to
   * reject instead). The shell surfaces it as a toast. */
  onAudioError?: (reason: string) => void;
  /** External-module entry importer. Defaults to a runtime dynamic `import()`;
   * a seam for unit tests (jsdom cannot import a served module URL), not a
   * production configuration point. */
  importModule?: (url: string) => Promise<unknown>;
}

/** One enabled external module resolved against the installed catalog, as built by
 * `WorldSession.#buildEntries` and consumed by `WorldSession.#recordLoaded`. Carries the
 * install FOLDER id alongside the `(manifest, entry)` pair `loadModules` itself consumes,
 * because that pair alone loses the folder id — the key space `#externalModuleIds` is diffed
 * against — the moment it's handed to `loadModules`. */
interface ResolvedModuleEntry {
  /** The canonical install-folder id (`InstalledModuleInfo.id`); the server's enabled-set key
   * space, distinct from `manifest.id` (see `WorldSession.#buildEntries`'s own doc). */
  folderId: string;
  /** The discovered manifest, re-validated by `loadModules` before import. */
  manifest: ModuleManifest;
  /** The importable specifier/URL passed to `loadModules`' `ImportFn`. */
  entry: string;
}

/** The legacy-prefix global shape older WebKit exposes instead of the standard
 * `AudioContext` constructor (`createDeviceAudioContext`'s fallback read). */
interface WebkitAudioGlobal {
  /** The prefixed constructor (absent on every modern engine). */
  webkitAudioContext?: typeof AudioContext;
}

/** Construct the device's `AudioContext` for `AudioEngineOpts.createContext`: the standard
 * constructor first, then the legacy webkit-prefixed one; when NEITHER exists the throw is
 * what routes `AudioEngine.unlock` onto its degraded bare-element mode.
 * @returns The new audio context.
 * @example
 * ```
 * // wired as `AudioEngineOpts.createContext` in `WorldSession`'s constructor — exercised
 * // through `@shadowcat/audio`'s engine tests with a stub context
 * ```
 */
function createDeviceAudioContext(): AudioContext {
  const Ctor = globalThis.AudioContext ?? (globalThis as WebkitAudioGlobal).webkitAudioContext;
  if (!Ctor) throw new Error("this device has no Web Audio API");
  return new Ctor();
}

/**
 * Per-world session controller: owns the WS connection, the authoritative
 * `DocumentStore` + optimistic view, first-party and external module
 * activation, and the scene/chat/move/search request delegates the shell
 * exposes through `AppContext`. One instance per entered world; `enter(worldId)`
 * opens the connection and `leave()` tears it down.
 * @example
 * ```
 * declare const selfId: string;
 * declare const connect: Connect;
 * declare const layoutModule: Module;
 * declare const worldId: string;
 * const session = new WorldSession({ selfId, connect, modules: [layoutModule] });
 * await session.enter(worldId);
 * ```
 */
export class WorldSession {
  /** Authoritative document store, fed by `onCommand`; the rollback base for the
   * optimistic view (`documents`). */
  readonly store = new DocumentStore();
  /** Registry of contract declarations + contributions from active modules; passed
   * into `ModuleRegistry` and exposed to `AppContext`. */
  readonly contributions = new ContributionRegistry();
  /** Resolves asset ids to URLs; bumped on `onAssetChanged`. */
  readonly assets = new AssetResolver();
  /** Canvas interaction bridge: the Stage attaches the engine; tool components reach
   * it via AppContext. Stable across Stage remount. */
  readonly sceneInteraction = new SceneInteractionBridge();
  /** The actor the place tool stamps; set by module-actors, read by scene-tools. Stable. */
  readonly actorSelection = new ActorSelection();
  /** Selected token ids for group-select; set by the factions panel, read by the select tool. Stable. */
  readonly tokenSelection = new TokenSelection();
  /** `onAssetChanged` subscriber set. */
  #assetListeners = new Set<(msg: AssetChangedNotice) => void>();
  /** `onPing` subscriber set. */
  #pingListeners = new Set<
    (msg: {
      /** The scene the ping was placed on. */
      scene: string;
      /** Scene-space x coordinate. */
      x: number;
      /** Scene-space y coordinate. */
      y: number;
      /** The user who placed the ping. */
      user: string;
    }) => void
  >();
  /** `onEmote` subscriber set. */
  #emoteListeners = new Set<
    (msg: {
      /** The scene the token stands on. */
      scene: string;
      /** The token the emote plays over. */
      token: string;
      /** The user who emoted. */
      user: string;
      /** The emote glyph(s). */
      emote: string;
    }) => void
  >();
  /** `onVfx` subscriber set. */
  #vfxListeners = new Set<(msg: VfxNotice) => void>();
  /** Listeners for THIS client's own `moveRequest` outcomes —
   * not a broadcast of every scene viewer's moves, unlike `#pingListeners`. */
  #moveOutcomeListeners = new Set<
    (msg: {
      /** The token that moved. */
      tokenId: string;
      /** The derived outcome; see `moveRequest`'s doc for how `executed`/`truncated`
       * are derived and how they differ from `MoveStream.truncated`. */
      outcome: "executed" | "truncated" | "rejected";
    }) => void
  >();
  /** Live `subscribeScene` records, keyed by a locally-generated subscription id. */
  #sceneSubs = new Map<string, SceneSubRecord>();
  /** The WS connection lifecycle; `"open"` after `enter()`'s `WsClient.start()`
   * resolves, `"closed"` after `leave()`. */
  state = $state<ConnState>("closed");
  /** This client's role in the entered world, set from the Welcome frame; `null`
   * before the first Welcome and after `leave()`. */
  role = $state<WorldRole | null>(null);
  /** The entered world's id, set at the start of `enter()`; `null` after `leave()`. */
  world = $state<string | null>(null);
  /** Client-local GM override of the rendered/subscribed scene ("GM roams"). Never set for
   * a player (they follow `world-settings.activeScene`). Overrides `viewedSceneId` for THIS
   * client's own render + vision + see-as channels only; the server is unaware of it. */
  #gmViewedScene = $state<string | null>(null);
  /** GM per-scene token-selection stash: `setGmViewedScene` moves the live `tokenSelection` into
   * this map (keyed by the scene being LEFT) before switching, and restores whatever was stashed
   * for the scene being ENTERED (empty if never selected there) — so roaming away and back
   * preserves a selection instead of leaking it across scenes or losing it. */
  #tokenSelectionByScene = new Map<string, Set<string>>();
  /** GM per-scene viewed-level override (`sessionState`'s `viewedLevel` map), loaded lazily per
   * scene on first read. Never set for a player (they follow `levelOf` of their primary token's
   * elevation). A `SvelteMap`, not a plain `Map` wrapped in `$state` — `$state` only deep-proxies
   * plain objects/arrays (`Map`'s prototype is neither), so an in-place `.set()` on a plain
   * `$state<Map<...>>` is invisible to Svelte's reactivity; `SvelteMap` is the reactive built-in
   * that makes a `$derived`/`$effect` reading `viewedLevel` (e.g. `LevelSwitcher`'s `active` prop)
   * re-run after a `setViewedLevel` call. */
  #gmViewedLevel = new SvelteMap<string, string | null>();
  /** The server's resolved token footprints, replaced wholesale by each `"footprints"` frame.
   * `$state` so every consumer — canvas reconcile, hit-test, the place tool —
   * re-reads the same authoritative extents the moment a frame lands. `EMPTY_FOOTPRINTS` until
   * the first frame, under which a token draws at its document's own authored `w`/`h`. */
  #footprints = $state<FootprintLookup>(EMPTY_FOOTPRINTS);
  /** Handle for the session-owned `"footprints"` subscription; dropped in `leave()` so a second
   * `enter()` does not stack a duplicate record. */
  #footprintsSub: SceneSubscription | null = null;
  /** The per-device mixer + one-shot/loop engine (`AppContext.audio`'s backing); constructed
   * once here (its wire-facing closures read `#ws` lazily, so a pre-`enter()` read is a safe
   * no-op) and disposed in `leave()`. */
  #audioEngine: AudioEngine;
  /** The audio reactivity bridge, created on the first `audio` read (one per session):
   * `AudioEngine.subscribe` behind a `createSubscriber`, the same bridge shape
   * `makeReactiveStore` wraps a `DocumentStore` in — every `channels`/`duck` read through
   * `AppContext.audio` re-runs its caller's derivation on a device-state change. */
  #audioSubscribe: ReturnType<typeof createSubscriber> | null = null;
  /** The audio-state document-store subscription driving `#audioEngine.applyState`; dropped
   * in `leave()`. */
  #audioUnsub: (() => void) | null = null;
  /** Handle for the session-owned `"combat"` subscription; dropped in `leave()` so a second
   * `enter()` does not stack a duplicate record. */
  #combatSub: SceneSubscription | null = null;
  /** Handle for the session-owned `"audibility"` subscription; dropped in `leave()` so a
   * second `enter()` does not stack a duplicate record. */
  #audibilitySub: SceneSubscription | null = null;
  /** The full multi-scene payload from the latest `"audibility"` frame — cached (not just
   * forwarded) so `setGmViewedScene` can re-derive and re-apply the newly-viewed scene's slice
   * immediately on a roam, without waiting for the next server push (the SAME reason
   * `#footprints` caches its own multi-scene lookup rather than discarding it after use). */
  #audibilityPayload: AudibilityPayload = EMPTY_AUDIBILITY;
  /** userId → username for the world's members, fetched on every role's Welcome
   * (chat author/whisper-recipient name resolution; the GM additionally uses it
   * for see-as labels). A stable reactive Map (mutated in place, never reassigned)
   * so the reference captured into AppContext at mount stays valid and consumers
   * re-render when it populates on (re)connect. */
  readonly members = new SvelteMap<string, string>();
  /** World-default capability grants + declarative requirements from the latest Welcome; inputs
   * to the advisory `canEdit` gate. Re-set on every (re)connect. `$state` so a capability-only
   * Welcome (no other reactive field changing) still refreshes every `canEdit`-gated
   * `{#if}` reading through it. */
  #worldGrants: WireWelcome["world_default_grants"] = $state({ by_role: {}, by_user: {} });
  /** Module-declared write-capability requirements from the latest Welcome; the
   * advisory-only half of `canEdit`'s `#requirements` caveat — see `canEdit`'s doc. `$state`
   * for the same reactivity reason as `#worldGrants`. */
  #requirements: WireCapabilityRequirement[] = $state([]);
  /** This connection's own projected world-level capabilities from the latest Welcome
   * (`role_capabilities`); input to `canCreate`. Re-set on every (re)connect. `$state` so a
   * capability-only Welcome refreshes every `canCreate`-gated `{#if}` reading through it. */
  #roleCaps: WireWelcome["role_capabilities"] = $state({ all: [], by_type: {} });

  /** The live transport, constructed fresh in `enter()` and dropped in `leave()`;
   * `null` before the first `enter()` and after `leave()`. */
  #ws: WsClient | null = null;
  /** The optimistic (predicted) document view backing `documents`. */
  #optimistic: OptimisticClient;
  /** Intents predicted while reconnecting (transport down but the client is still
   * `running`), queued to flush in FIFO order after the next resync completes. */
  #offlineQueue: {
    /** The correlated intent id, matched against the server echo/reject. */
    intentId: string;
    /** The operations predicted + queued as one intent. */
    ops: WireOperation[];
  }[] = [];
  /** The optimistic (predicted) document view — the canvas render source, so a placed
   * or dragged document shows immediately. `store` stays the authoritative rollback base
   * (panels that want confirmed-only state read it).
   * @returns The optimistic client for this session. */
  get documents(): OptimisticClient {
    return this.#optimistic;
  }

  /** The current user's id (ownership checks).
   * @returns This client's own user id. */
  get selfId(): string {
    return this.opts.selfId;
  }

  /** The scene THIS client renders + subscribes to. A GM's local roam
   * (`#gmViewedScene`) overrides; otherwise follows `world-settings.activeScene`, else the first
   * scene. Reads the optimistic view + `#gmViewedScene` $state, so Svelte deriveds that read it
   * (bridged through `documents.subscribe`) react to both scene-doc changes and roam changes.
   * @returns The scene id this client renders/subscribes to, or `null` when the world has no scene yet. */
  get viewedSceneId(): string | null {
    return resolveViewedScene(this.#optimistic, { gmViewedScene: this.role === "gm" ? this.#gmViewedScene : null });
  }

  /** The server's resolved token footprints. There is no client-side footprint formula: the extent
   * the canvas draws and the hit-test picks is the one the server's
   * movement gate collides with, read off the `"footprints"` derived channel.
   * @returns The current lookup; `EMPTY_FOOTPRINTS` before the first frame. */
  get footprints(): FootprintLookup {
    return this.#footprints;
  }

  /** The per-device audio seam (`AppContext.audio`). `AudioEngine` implements `AudioApi`
   * directly for everything but `setChannel`/`duck.setDepth`: those two additionally persist to
   * this device's `shadowcat.audio` mirror — a `localStorage` dependency `AudioEngine` itself
   * deliberately does not have, staying framework/platform-neutral — so this getter wraps
   * exactly those two surfaces. (`listenAs`, like `transport`, is the engine's own forwarder to
   * the `AudioEngineOpts` seam this session wires to `WsClient`.)
   * @returns The audio API the shell publishes on `AppContext.audio`. */
  get audio(): AudioApi {
    const engine = this.#audioEngine;
    const subscribeAudio = (this.#audioSubscribe ??= createSubscriber((update) =>
      engine.subscribe(update),
    ));
    return {
      get channels() {
        subscribeAudio();
        return engine.channels;
      },
      setChannel: (id, patch) => {
        engine.setChannel(id, patch);
        if (typeof localStorage !== "undefined") {
          writeAudioMirror(localStorage, { channels: engine.channels, duckDepth: engine.duck.depth });
        }
      },
      unlock: () => engine.unlock(),
      get duck() {
        const duck = engine.duck;
        return {
          addSource: (id: string) => duck.addSource(id),
          removeSource: (id: string) => duck.removeSource(id),
          get gain() {
            subscribeAudio();
            return duck.gain;
          },
          get depth() {
            subscribeAudio();
            return duck.depth;
          },
          setDepth: (depth: number) => {
            duck.setDepth(depth);
            if (typeof localStorage !== "undefined") {
              writeAudioMirror(localStorage, { channels: engine.channels, duckDepth: duck.depth });
            }
            engine.notifyAudioChanged();
          },
        };
      },
      playOneShot: (asset, opts) => engine.playOneShot(asset, opts),
      serverNow: () => engine.serverNow(),
      transport: (op) => engine.transport(op),
      listenAs: (token) => engine.listenAs(token),
    };
  }

  /** GM local roam: view any scene without moving players. Ignored (warned) for a non-GM —
   * players have no local override. `null` clears the roam (follow `activeScene`).
   * @param id The scene to roam to, or `null` to resume following `activeScene`.
   * @example
   * ```
   * declare const session: WorldSession;
   * declare const otherSceneId: string | null;
   * session.setGmViewedScene(otherSceneId); // GM only; no-op+warns for a player
   * ```
   */
  setGmViewedScene(id: string | null): void {
    if (this.role !== "gm") {
      this.#logger.warn("setGmViewedScene ignored: caller is not a GM");
      return;
    }
    const leaving = this.viewedSceneId;
    if (leaving) this.#tokenSelectionByScene.set(leaving, new Set(this.tokenSelection.ids));
    this.#gmViewedScene = id;
    const entering = this.viewedSceneId;
    this.tokenSelection.set(entering ? (this.#tokenSelectionByScene.get(entering) ?? []) : []);
    // A roam carries no new server frame: re-apply the newly-viewed scene's slice from the
    // already-cached multi-scene payload immediately, rather than waiting for the next push.
    this.#audioEngine.applyAudibility(sceneAudibility(this.#audibilityPayload, entering));
  }

  /** The FIRST token (lowest `id`, for determinism) in `scene` whose `owner` equals `selfId` —
   * an advisory, client-only read mirroring the server's `token_effective_owner` override-first
   * rule at the shallow level; never authoritative. Used to derive a player's `viewedLevel` from
   * their own token's elevation.
   * @param scene The scene to search.
   * @returns The resolved token document, or `null` if the caller owns none in `scene`.
   * @example
   * ```
   * declare const scene: string;
   * // called from the viewedLevel getter; not part of the public API
   * this.#primaryTokenIn(scene);
   * ```
   */
  #primaryTokenIn(scene: string): WireDocument | null {
    const owned = this.#optimistic
      .query("token")
      .filter((t) => t.parent_id === scene && t.owner === this.selfId)
      .sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
    return owned[0] ?? null;
  }

  /** The level this client renders/subscribes to for the CURRENTLY viewed scene. A GM reads
   * `#gmViewedLevel`'s stash for that scene (seeded from `getViewedLevel` on first access); a
   * player follows `levelOf` of their primary token's elevation over the viewed scene's `levels`.
   * `null` for a level-less scene.
   * @returns The viewed level id, or `null`. */
  get viewedLevel(): string | null {
    const sceneId = this.viewedSceneId;
    if (sceneId === null) return null;
    const sceneDoc = this.#optimistic.query("scene").find((s) => s.id === sceneId);
    const levels = (sceneDoc?.engine as SceneEngine | undefined)?.levels ?? [];
    if (levels.length === 0) return null;
    if (this.role === "gm") {
      if (!this.#gmViewedLevel.has(sceneId)) {
        this.#gmViewedLevel.set(sceneId, getViewedLevel(this.world ?? "", sceneId));
      }
      const stashed = this.#gmViewedLevel.get(sceneId) ?? null;
      return levels.some((l) => l.id === stashed) ? stashed : (levels[0]?.id ?? null);
    }
    const primary = this.#primaryTokenIn(sceneId);
    const elevation = primary ? ((primary.engine as TokenEngine | undefined)?.elevation ?? 0) : 0;
    return levelOf(levels, elevation)?.id ?? null;
  }

  /** GM local viewed-level override for the current scene; ignored (warned) for a non-GM —
   * a player instead follows their primary token's level.
   * @param id The level to view, or `null` to clear to the scene's first level.
   * @example
   * ```
   * declare const session: WorldSession;
   * session.setViewedLevel("l2"); // GM only; no-op+warns for a player
   * ```
   */
  setViewedLevel(id: string | null): void {
    if (this.role !== "gm") {
      this.#logger.warn("setViewedLevel ignored: caller is not a GM");
      return;
    }
    const sceneId = this.viewedSceneId;
    if (sceneId === null) return;
    this.#gmViewedLevel.set(sceneId, id);
    if (this.world) persistViewedLevel(this.world, sceneId, id);
  }

  /** Live full-text search over documents (subscription seam). Ephemeral: NOT re-established
   * across reconnects (unlike `subscribeScene`) — the caller re-subscribes on the next query.
   * Rejects immediately when there is no live transport.
   * @param query The FTS query string.
   * @param opts Subscription options.
   * @param opts.limit Maximum number of hits to return.
   * @param opts.timeoutMs Timeout for the initial subscribe round-trip.
   * @param opts.docTypes Narrows the ranked candidates to the listed doc_types; empty/omitted
   * = every type.
   * @param onUpdate Called with the current hit set whenever the live results change.
   * @returns A handle that resolves once the subscription is established; call
   * `unsubscribe()` on it to stop receiving updates.
   * @example
   * ```
   * declare const session: WorldSession;
   * declare function render(hits: WireSearchHit[]): void;
   * const handle = await session.searchDocuments("goblin", { limit: 20 }, (hits) => render(hits));
   * handle.unsubscribe();
   * ```
   */
  searchDocuments(
    query: string,
    opts: {
      /** See the `@param opts.limit` doc above. */
      limit?: number;
      /** See the `@param opts.timeoutMs` doc above. */
      timeoutMs?: number;
      /** See the `@param opts.docTypes` doc above. */
      docTypes?: string[];
    },
    onUpdate: (hits: WireSearchHit[]) => void,
  ): Promise<SubscriptionHandle> {
    if (!this.#ws) return Promise.reject(new Error("not connected"));
    return this.#ws.subscribeSearch(query, opts, onUpdate);
  }

  /** Advisory client-side mirror of the server's Update-path check, for showing/hiding write
   * controls. GM bypasses; the server remains authoritative and rejects a bypass at apply_intent.
   *
   * Caveat (`#requirements`): the Welcome union mixes GM-authored world_cap_requirements
   * with module-declared manifest requirements. For GM-authored entries this mirror matches
   * server enforcement exactly. Module-published entries are advisory UX only — the server
   * does NOT reject a write solely because a module declared a requirement on that path, so
   * this gate can be stricter here than the server actually is for module-only requirements.
   *
   * Caveat (`gm_role`): the `role === "gm"` short-circuit immediately below returns `true`
   * unconditionally and never consults `doc.permissions.gm_role`. The server's GM bypass is
   * conditional — a document carrying `gm_role: Some(role)` floors even a GM to an ordinary
   * `DocRole` resolution instead of the unconditional grant (`effective_role`/`resolve_access`)
   * — so this gate's write affordances can over-permit on a
   * `gm_role`-capped document. Advisory-only, not a live bug today: `Repository::apply_intent`
   * re-checks independently, and separately rejects every ordinary client Update to a `message` doc
   * outright regardless of role. `build_message_doc` is the only place the SERVER
   * constructs a `gm_role` today — NOT a bound on where it can live: it is an ordinary field on
   * every document's `permissions` block, so do not assume it is chat-specific (see the SCOPE
   * NOTE on `canWritePath` in `@shadowcat/core`).
   *
   * @param doc The document being edited.
   * @param path The JSON-pointer path within `doc` the caller wants to write.
   * @returns Whether write controls for `path` should be shown to this user.
   * @example
   * ```
   * declare const session: WorldSession;
   * declare const doc: WireDocument;
   * declare function showHpInput(): void;
   * if (session.canEdit(doc, "/system/hp")) showHpInput();
   * ```
   */
  canEdit(doc: WireDocument, path: string): boolean {
    if (this.role === "gm") return true;
    const role = this.role;
    if (!role) return false;
    return canWritePath(path, this.#capsFor(doc, role), false, this.#requirements);
  }

  /** The caller's resolved capability set on `doc` — the shared resolution `canEdit` and
   * `canDelete` both build on. Effective ownership (a linked token inherits its actor's owner)
   * floors the caller at DocRole.Owner, mirroring the server's `effective_role` —
   * token-scoped there, so token-scoped here (`ownerFloorApplies`). Resolved from the
   * OPTIMISTIC view so a just-reassigned owner gates controls without waiting for the echo.
   * @param doc The document to resolve capabilities against.
   * @param role The caller's non-GM world role (callers have already handled the GM/unresolved
   * cases before reaching here).
   * @returns The resolved capability set.
   * @example
   * ```
   * // private helper; not part of the public API — see canEdit/canDelete
   * declare const doc: WireDocument;
   * this.#capsFor(doc, "player");
   * ```
   */
  #capsFor(doc: WireDocument, role: WorldRole): Set<string> {
    const owned = ownerFloorApplies(doc, this.opts.selfId, this.#optimistic);
    return resolveCaps(doc.permissions, this.opts.selfId, role, this.#worldGrants, owned);
  }

  /** Advisory mirror of the server's `Operation::Delete` gate: whether this caller may
   * delete `doc`. A GM may always delete; an unresolved role (not yet connected) may
   * never; otherwise the resolved capability set (see `#capsFor`) must hold
   * `core:delete` — never derived from `doc.owner` (see `grantAuthor`'s doc: the
   * DocRole `owner` grant, not the `owner` field, is what carries delete authority).
   * @param doc The document to check.
   * @returns Whether this caller may delete `doc`.
   * @example
   * ```
   * declare const session: WorldSession;
   * declare const doc: WireDocument;
   * declare function showDeleteButton(): void;
   * if (session.canDelete(doc)) showDeleteButton();
   * ```
   */
  canDelete(doc: WireDocument): boolean {
    if (this.role === "gm") return true;
    const role = this.role;
    if (!role) return false;
    return this.#capsFor(doc, role).has("core:delete");
  }

  /** Advisory mirror of the server's `core:create` policy (`WorldCapDefaults::role_has`,
   * consulted by `apply_intent`): whether this caller may create a document of `docType`.
   * @param docType The document's `doc_type`.
   * @returns Whether the create is advisory-permitted.
   * @example
   * ```
   * declare const session: WorldSession;
   * declare function showNewNoteButton(): void;
   * if (session.canCreate("note")) showNewNoteButton();
   * ```
   */
  canCreate(docType: string): boolean {
    const role = this.role;
    if (!role) return false;
    return canCreateDoc(docType, role, this.#roleCaps);
  }
  /** Registry of first-party + external modules; `activate()` is called from
   * `#onWelcome`. */
  #modules: ModuleRegistry;
  /** The shared hook bus every module's `hooks` wrapper delegates to, and the bus
   * `#combatEmitter` emits `combat:*` events on. Held directly (not read back off
   * `ModuleRegistry`, which does not expose its `Deps`) so `defineCombatHooks` can declare
   * against it before any module registers a listener. */
  #hooks: HookBus;
  /** The shared service registry every module's `services` wrapper delegates to; also where
   * `#combat` is provided under `COMBAT_SERVICE` for module lookup. */
  #services: ServiceRegistry;
  /** The combat seam controller, exposed to Svelte via `get combat()` and to every module via
   * `COMBAT_SERVICE`. Constructed once per session (before any world is entered); its `world`/
   * `role` deps read `this.world`/`this.role` live rather than snapshotting them. */
  #combat: CombatController;
  /** Chains `combat:*` hook emissions from `onCommand`'s derived events onto a strict-order
   * queue; see `CombatHookEmitter`'s own doc. */
  #combatEmitter: CombatHookEmitter;
  /** Diagnostics sink for this session; `opts.logger` or a console default. */
  #logger: Logger;
  /** In-world bootstrap guards. Modules are ADDED exactly once per session
   * (re-adding would duplicate registrations). `#activated` is set
   * synchronously before the `activate()` call (so a Welcome arriving while a
   * prior one is still mid-activation cannot re-enter and double-activate),
   * but reverts to `false` if `activate()` throws — a thrown activation (e.g.
   * a contract cycle) is therefore re-attempted on the next Welcome instead of
   * being cached for the session's life with every Surface silently empty.
   * `ModuleRegistry.activate` is incremental (activates only not-yet-active
   * modules), so a retry never double-activates an already-active module. */
  #modulesAdded = false;
  /** Latches only on a SUCCESSFUL `activate()`; reverted to `false` in the catch on
   * a thrown activation so the next Welcome retries — see the field-level doc above. */
  #activated = false;
  /** The connected server's version, captured at Welcome for `reconcileInstalledModules`'s later
   * use (the original `#loadExternalModules` call already has it as a local; a live reconcile
   * triggered later — e.g. from `ModuleManager.svelte`'s save() — needs its own copy). */
  #serverVersion: string | undefined;
  /** Every EXTERNAL (community) module currently loaded into `#modules`, keyed on the canonical
   * install FOLDER id (`InstalledModuleInfo.id` — the same key space `getEnabledModules`/
   * `listInstalledModules` use) and mapped to that module's own declared manifest id — first-party
   * modules (`opts.modules`) are never tracked here. Two distinct id spaces, both load-bearing:
   * `reconcileInstalledModules` diffs the FOLDER-id keys against a freshly-fetched enabled set
   * (which is folder-id-keyed), but `ModuleRegistry.unload` operates on `module.manifest.id`
   * (what `ModuleRegistry.add` actually keys the module under) — a folder id and its manifest id
   * may legitimately differ (see `#buildEntries`'s own doc), so unloading by the wrong one either
   * throws (module not found) or silently no-ops. */
  #externalModuleIds = new Map<string, string>();
  /** The `<link>` element carrying each loaded external module's declared
   * stylesheet (`ModuleManifest.style`), keyed by manifest id. Removed on
   * unload (reconcile) and in bulk on `leave()` — a link left behind would
   * keep a previous world's module styles applied app-wide. */
  #moduleStyleLinks = new Map<string, HTMLLinkElement>();

  /** Construct a session bound to one connection factory + default module set; call
   * `enter(worldId)` to open the world connection.
   * @param opts Connection factory, default modules, and diagnostics/eviction callbacks.
   * @example
   * ```
   * declare const selfId: string;
   * declare const connect: Connect;
   * declare const layoutModule: Module;
   * const session = new WorldSession({ selfId, connect, modules: [layoutModule] });
   * ```
   */
  constructor(private readonly opts: WorldSessionOpts) {
    this.#logger = opts.logger ?? consoleLogger();
    this.#optimistic = new OptimisticClient(opts.selfId, this.#logger);
    this.#hooks = new HookBus(this.#logger);
    defineCombatHooks(this.#hooks);
    this.#services = new ServiceRegistry();
    this.#combat = new CombatController({
      documents: this.#optimistic,
      dispatchIntent: (ops) => this.dispatchIntent(ops),
      sendCombat: (m) => (this.#ws ? this.#ws.combat(m) : Promise.reject(new Error("not connected"))),
      selfId: opts.selfId,
      role: () => (this.role === "gm" ? "gm" : this.role ? "player" : null),
      canEdit: (doc, path) => this.canEdit(doc, path),
      // `enter(worldId)` sets `this.world` before any document mutation can reach this
      // controller. Before `enter()` (or after `leave()`), `dispatchIntent` is a GUARDED DROP —
      // it warns and discards, returning `false` — and the controller turns that `false` into a
      // thrown `CombatClientError`, so an op built against the empty-string fallback is never
      // transmitted and no fabricated id is ever returned for a discarded op.
      world: () => this.world ?? "",
      logger: this.#logger,
    });
    this.#combatEmitter = new CombatHookEmitter(this.#hooks, this.#logger);
    this.#services.provide(COMBAT_SERVICE, this.#combat, { version: COMBAT_HOOK_VERSION });
    const mirror = typeof localStorage !== "undefined" ? readAudioMirror(localStorage) : undefined;
    this.#audioEngine = new AudioEngine({
      resolver: this.assets,
      serverNow: () => this.#ws?.serverNow() ?? 0,
      transport: (op) => this.#ws?.audioTransport(op),
      listenAs: (token) => this.#ws?.audioListenAs(token),
      createContext: createDeviceAudioContext,
      onTrackEnded: (id) => this.#ws?.audioTransport({ type: "track_ended", id }),
      fadeMsFor: (playlistId) => {
        if (!playlistId) return 0;
        const doc = this.documents.query("playlist").find((d) => d.id === playlistId);
        /** The playlist document's engine body (only `fadeMs` is read here). */
        const engine = doc?.engine as {
          /** The playlist's crossfade duration, ms. */
          fadeMs?: number;
        } | undefined;
        return engine?.fadeMs ?? 0;
      },
      duckDepth: mirror?.duckDepth ?? DEFAULT_DUCK_DEPTH,
      raf: (cb) => requestAnimationFrame(cb),
      caf: (handle) => cancelAnimationFrame(handle),
      spatial: () => performanceController.current.spatialAudio,
    });
    setMediaElementFactory(() => document.createElement("audio"));
    if (mirror) {
      for (const [id, state] of Object.entries(mirror.channels)) {
        this.#audioEngine.setChannel(id as AudioChannelId, state);
      }
    }
    this.#modules = new ModuleRegistry({
      hooks: this.#hooks,
      services: this.#services,
      middleware: new MiddlewareChain(),
      store: this.store,
      client: this.#optimistic,
      logger: this.#logger,
      contributions: this.contributions,
      i18n,
    });
  }

  /** The combat seam: reads over the optimistic view, server-resolved resource numbers, the
   * server-owned clock's intents, and document helpers. See `CombatApi`'s own doc.
   * @returns This session's combat controller. */
  get combat(): CombatApi {
    return this.#combat;
  }

  /** Predict `ops` optimistically and transmit them as one correlated Intent. The
   * single `intent_id` ties the local prediction to the server echo/reject (FIFO
   * confirm). ORDERING: `applyIntent` runs BEFORE `ws.send` on every path that
   * predicts at all — the optimistic view is updated first, so a synchronous reader
   * observing the send has already seen the prediction, and a send that throws cannot
   * leave a transmitted-but-unpredicted op. While reconnecting (transport down but `running`), predict AND queue:
   * every offline intent queues, so optimistic FIFO order equals the eventual send
   * order and the confirm-correlation contract holds. A flush happens after resync
   * (the optimistic view rebases onto authoritative state first). When stopped, drop
   * without an orphaned pending entry.
   * @param ops The operations to predict + transmit as one intent.
   * @returns `true` when `ops` were predicted and transmitted or queued; `false` when they were
   * dropped (no socket, or a stopped one) — the one signal a caller that must not silently lose
   * a write (`CombatController`) turns into an explicit failure.
   * @example
   * ```
   * declare const session: WorldSession;
   * declare const docId: string;
   * session.dispatchIntent([
   *   { op: "update", doc_id: docId, changes: [{ path: "/system/hp", old: 8, new: 10 }] },
   * ]);
   * ```
   */
  dispatchIntent(ops: WireOperation[]): boolean {
    const intentId = crypto.randomUUID();
    if (this.#ws?.connected) {
      this.#optimistic.applyIntent(intentId, ops);
      this.#ws.send({ type: "intent", intent_id: intentId, ops });
      return true;
    }
    if (this.#ws?.running) {
      // Reconnecting: predict now (immediate feedback) and queue for FIFO replay.
      this.#optimistic.applyIntent(intentId, ops);
      this.#offlineQueue.push({ intentId, ops });
      return true;
    }
    // Stopped (or no socket): no reconnect is coming, so drop without predicting —
    // an orphaned pending entry would mis-correlate the next live echo.
    this.#logger.warn("dropping intent: world session stopped");
    return false;
  }

  /** Transmit intents queued while offline, in FIFO order. Called after a resync
   * completes (authoritative state is current and the optimistic view has rebased),
   * so the already-predicted intents converge as their echoes confirm them.
   * @example
   * ```
   * // called from onResyncComplete; not part of the public API
   * this.#flushOfflineQueue();
   * ```
   */
  #flushOfflineQueue(): void {
    if (!this.#ws?.connected || this.#offlineQueue.length === 0) return;
    const queued = this.#offlineQueue;
    this.#offlineQueue = [];
    for (const { intentId, ops } of queued) {
      // Prediction was applied at dispatch; only transmit, preserving order.
      this.#ws.send({ type: "intent", intent_id: intentId, ops });
    }
  }

  /** Subscribe to asset replace/delete notices; returns an unsubscribe.
   * @param cb Called with the changed asset's id and the change kind.
   * @returns A function that removes this listener.
   * @example
   * ```
   * declare const session: WorldSession;
   * declare const resolver: AssetResolver;
   * const off = session.onAssetChanged((msg) => resolver.onAssetChanged(msg));
   * off();
   * ```
   */
  onAssetChanged(cb: (msg: AssetChangedNotice) => void): () => void {
    this.#assetListeners.add(cb);
    return () => this.#assetListeners.delete(cb);
  }

  /** Subscribe to relayed location pings (incl. our own echo); returns an unsubscribe.
   * @param cb Called with the scene, coordinates, and originating user of each ping.
   * @returns A function that removes this listener.
   * @example
   * ```
   * declare const session: WorldSession;
   * declare function renderPing(scene: string, x: number, y: number, user: string): void;
   * const off = session.onPing(({ scene, x, y, user }) => renderPing(scene, x, y, user));
   * off();
   * ```
   */
  onPing(
    cb: (msg: {
      /** The scene the ping was placed on. */
      scene: string;
      /** Scene-space x coordinate. */
      x: number;
      /** Scene-space y coordinate. */
      y: number;
      /** The user who placed the ping. */
      user: string;
    }) => void,
  ): () => void {
    this.#pingListeners.add(cb);
    return () => this.#pingListeners.delete(cb);
  }

  /** Subscribe to relayed emotes (incl. our own echo); returns an unsubscribe.
   * @param cb Called with the scene, token, originating user, and glyph(s) of each emote.
   * @returns A function that removes this listener.
   * @example
   * ```
   * declare const session: WorldSession;
   * declare function renderEmote(token: string, emote: string): void;
   * const off = session.onEmote(({ token, emote }) => renderEmote(token, emote));
   * off();
   * ```
   */
  onEmote(
    cb: (msg: {
      /** The scene the token stands on. */
      scene: string;
      /** The token the emote plays over. */
      token: string;
      /** The user who emoted. */
      user: string;
      /** The emote glyph(s). */
      emote: string;
    }) => void,
  ): () => void {
    this.#emoteListeners.add(cb);
    return () => this.#emoteListeners.delete(cb);
  }

  /** Broadcast a transient location ping at scene coords on the currently-viewed scene
   * (`viewedSceneId`: a GM's local roam override, else the followed `activeScene`). No-op when
   * disconnected or no scene exists; the server relays it back to all members (incl. us).
   * @param x Scene-space x coordinate to ping.
   * @param y Scene-space y coordinate to ping.
   * @example
   * ```
   * declare const session: WorldSession;
   * declare const cellX: number;
   * declare const cellY: number;
   * session.sendPing(cellX, cellY);
   * ```
   */
  sendPing(x: number, y: number): void {
    const sceneId = this.viewedSceneId;
    if (!sceneId) return;
    this.#ws?.send({ type: "scene_ping", scene: sceneId, x, y });
  }

  /** Broadcast a transient emote over `token` on the currently-viewed scene
   * (`viewedSceneId`, same target `sendPing` uses). No-op when disconnected or no scene
   * exists; the server relays it back to all members (incl. us) after re-authorizing
   * effective ownership, so an over-reaching send drops silently.
   * @param token The token document id to emote over.
   * @param emote The emote glyph(s); the server bounds this to 1..=16 bytes.
   * @example
   * ```
   * declare const session: WorldSession;
   * declare const tokenId: string;
   * session.sendEmote(tokenId, "😀");
   * ```
   */
  sendEmote(token: string, emote: string): void {
    const sceneId = this.viewedSceneId;
    if (!sceneId) return;
    this.#ws?.send({ type: "emote", scene: sceneId, token, emote });
  }

  /** Subscribe to relayed VFX one-shots (incl. our own echo); returns an unsubscribe.
   * @param cb Called with each one-shot's scene, position, asset, sending user, and id.
   * @returns A function that removes this listener.
   * @example
   * ```
   * declare const session: WorldSession;
   * declare function playVfxLocally(msg: VfxNotice): void;
   * const off = session.onVfx(playVfxLocally);
   * off();
   * ```
   */
  onVfx(cb: (msg: VfxNotice) => void): () => void {
    this.#vfxListeners.add(cb);
    return () => this.#vfxListeners.delete(cb);
  }

  /** Broadcast a one-shot VFX playback request. No-op when disconnected. The server
   * re-authorizes scene readability and world role (spectator refused) and drops an
   * over-reaching send silently, so callers may offer this client-advisory only. The request
   * carries an explicit `scene` (unlike `sendPing`/`sendEmote`'s auto-derived target): a
   * caller may legitimately need to name a scene other than its own currently-viewed one (a
   * portal effect plays at both ends of a teleport), so the caller supplies it, matching
   * `pathfind`/`moveRequest`'s explicit-scene convention.
   * @param req The one-shot request.
   * @example
   * ```
   * declare const session: WorldSession;
   * session.playVfx({ scene: "s1", asset: "a1", x: 0, y: 0 });
   * ```
   */
  playVfx(req: VfxPlayRequest): void {
    this.#ws?.playVfx(req);
  }

  /** Request a grid A* path on the server. Thin delegate to `WsClient.pathfind`;
   * rejects immediately when there is no live transport.
   * @param scene The scene to path on.
   * @param start The mover's current position.
   * @param waypoints Ordered points the path must visit, in order.
   * @param footprintRadius Hypothetical mover footprint radius; ignored server-side when
   * `token` is given (the server derives the footprint from the token instead).
   * @param token Optional token id the route is for; when present, overrides `footprintRadius`.
   * @returns The computed path.
   * @example
   * ```
   * declare const session: WorldSession;
   * declare const sceneId: string;
   * declare const tokenId: string;
   * // hypothetical preview (no token): footprintRadius is honored
   * const preview = await session.pathfind(sceneId, [0, 0], [[5, 5]], 0.5);
   * // for a real token: the server derives the footprint from it; footprintRadius is ignored
   * const forToken = await session.pathfind(sceneId, [0, 0], [[5, 5]], 0.5, tokenId);
   * ```
   */
  pathfind(
    scene: string,
    start: [number, number],
    waypoints: [number, number][],
    footprintRadius: number,
    token?: string,
  ): Promise<PathResult> {
    if (!this.#ws) return Promise.reject(new Error("not connected"));
    return this.#ws.pathfind(scene, start, waypoints, footprintRadius, token);
  }

  /** Request server-authoritative move execution for `tokenId` along `path` on
   * `scene`. Resolves with the broadcast `MoveStream` when the server confirms;
   * rejects immediately when there is no live transport. Animation is broadcast-driven
   * via `onMoveStream` for all scene viewers; the resolve value signals success only.
   * @param scene The scene the token moves on.
   * @param tokenId The token to move.
   * @param path The requested waypoints, in order.
   * @returns The broadcast `MoveStream` confirming the move (see `onMoveOutcome` for a
   * derived executed/truncated/rejected signal).
   * @example
   * ```
   * declare const session: WorldSession;
   * declare const sceneId: string;
   * declare const tokenId: string;
   * const stream = await session.moveRequest(sceneId, tokenId, [[1, 1], [2, 2]]);
   * ```
   */
  moveRequest(
    scene: string,
    tokenId: string,
    path: [number, number][],
  ): Promise<MoveStream> {
    if (!this.#ws) return Promise.reject(new Error("not connected"));
    const p = this.#ws.moveRequest(scene, tokenId, path);
    // An observability signal derived from THIS SAME promise without altering its
    // resolution for the caller. Derived from geometry, NOT from the frame's `truncated`
    // flag, because the two answer different questions: `stream.stop` is the mover's own
    // exact, unclipped resting position, compared against the requested goal.
    // "executed" = stop reached the requested goal POSITION. Note this is reached-goal, not
    // "not truncated": a region ARREST landing exactly on the final cell also sets the
    // server's `MoveOutcome.truncated = true` while `stop` still equals the goal (`execute_move`:
    // arrest stops AT cell entry, so `stop_index == path.len()-1` on a final-step arrest) — the
    // token DID reach the goal, only further movement from there is barred, so this reads
    // "executed" by design (see the dedicated regression test below for this exact case).
    // A consumer wanting "was the traversal cut short", arrest-at-goal INCLUDED, must read
    // `MoveStream.truncated` instead — the authoritative flag, which no geometry can
    // reconstruct. It is `null` for a clipped observer and populated only for the mover/GM.
    // "truncated" = stop landed SHORT of the goal — a wall/mask/impassable-region gate cut the
    // move off before it arrived.
    const goal = path.at(-1) ?? null;
    p.then(
      (stream) => {
        const executed = goal !== null && stream.stop[0] === goal[0] && stream.stop[1] === goal[1];
        for (const cb of this.#moveOutcomeListeners) cb({ tokenId, outcome: executed ? "executed" : "truncated" });
      },
      // "rejected" covers every way this promise can fail to resolve with a MoveStream: a
      // genuine server `MoveError`, the 10s correlated-request timeout, or a transport
      // disconnect — not exclusively "the server refused the move". Fail-safe by construction:
      // a legal move whose confirmation is merely lost/delayed surfaces as "rejected", never
      // as a false "executed".
      () => {
        for (const cb of this.#moveOutcomeListeners) cb({ tokenId, outcome: "rejected" });
      },
    );
    return p;
  }

  /** Subscribe to THIS client's own `moveRequest` outcomes (executed/truncated/rejected) —
   * a read-only observability signal, not a broadcast of every scene viewer's moves (that's
   * `onMoveStream`, consumed internally for animation). Returns an unsubscribe.
   * @param cb Called with the moved token and its derived outcome.
   * @returns A function that removes this listener.
   * @example
   * ```
   * declare const session: WorldSession;
   * declare function log(tokenId: string, outcome: "executed" | "truncated" | "rejected"): void;
   * const off = session.onMoveOutcome(({ tokenId, outcome }) => log(tokenId, outcome));
   * off();
   * ```
   */
  onMoveOutcome(
    cb: (msg: {
      /** The token that moved. */
      tokenId: string;
      /** The derived outcome — see `moveRequest`'s doc for how `executed`/
       * `truncated` are derived and how they differ from `MoveStream.truncated`. */
      outcome: "executed" | "truncated" | "rejected";
    }) => void,
  ): () => void {
    this.#moveOutcomeListeners.add(cb);
    return () => this.#moveOutcomeListeners.delete(cb);
  }

  /** Send a chat message. Resolves/rejects with the correlated outcome; rejects
   * immediately when there is no live transport (the caller surfaces it).
   * @param opts The message to send; see {@link ChatSendOptions}.
   * @returns Resolves when `CHAT_ERROR_WINDOW_MS` elapses with no correlated `chat_error` —
   * success is ASSUMED from silence, not acknowledged: the server sends no ack frame and the
   * broadcast Event carries no correlation back to this promise. Rejects with the server's
   * player-presentable reason on a correlated `chat_error`, or on disconnect (the op's fate is
   * unknown, so it rejects rather than resolving silently).
   * @example
   * ```
   * declare const session: WorldSession;
   * await session.sendChatMessage({ channel: "main", content: "Hello!" });
   * ```
   */
  sendChatMessage(opts: ChatSendOptions): Promise<void> {
    if (!this.#ws) return Promise.reject(new Error("not connected"));
    return this.#ws.sendChatMessage(opts);
  }

  /** Edit an existing chat message. Resolves/rejects with the correlated outcome.
   * @param messageId The message to edit.
   * @param content The new raw (unsanitized) message text.
   * @returns Same silence-based resolution as `sendChatMessage` — resolves when the error window
   * elapses with no correlated `chat_error`, rejects on one or on disconnect.
   * @example
   * ```
   * declare const session: WorldSession;
   * declare const messageId: string;
   * await session.editChatMessage(messageId, "Updated text");
   * ```
   */
  editChatMessage(messageId: string, content: string): Promise<void> {
    if (!this.#ws) return Promise.reject(new Error("not connected"));
    return this.#ws.editChatMessage(messageId, content);
  }

  /** Delete an existing chat message. Resolves/rejects with the correlated outcome.
   * @param messageId The message to delete.
   * @returns Same silence-based resolution as `sendChatMessage` — resolves when the error window
   * elapses with no correlated `chat_error`, rejects on one or on disconnect.
   * @example
   * ```
   * declare const session: WorldSession;
   * declare const messageId: string;
   * await session.deleteChatMessage(messageId);
   * ```
   */
  deleteChatMessage(messageId: string): Promise<void> {
    if (!this.#ws) return Promise.reject(new Error("not connected"));
    return this.#ws.deleteChatMessage(messageId);
  }

  /** GM-only roll correction. Resolves/rejects with the correlated outcome.
   * @param messageId The message carrying the targeted roll.
   * @param rollId The targeted roll's stable id.
   * @param ops The targeted mutation(s) to apply.
   * @returns Same silence-based resolution as `sendChatMessage` -- resolves when the error window
   * elapses with no correlated `chat_error`, rejects on one or on disconnect.
   * @example
   * ```
   * declare const session: WorldSession;
   * await session.recalcRoll("msg-1", "roll-1", [{ kind: "remove_dice", ids: [2] }]);
   * ```
   */
  recalcRoll(messageId: string, rollId: string, ops: WireRecalcOp[]): Promise<void> {
    if (!this.#ws) return Promise.reject(new Error("not connected"));
    return this.#ws.recalcRoll(messageId, rollId, ops);
  }

  /** Send a merge intent (`merge_pull`/`merge_push`/`merge_revert`). Thin delegate to
   * `WsClient.merge`; rejects immediately when there is no live transport.
   * @param msg The merge frame to send, already carrying its own `request_id`.
   * @param opts Request options; `timeoutMs` bounds the wait for the correlated reply (the
   * caller sizes it to the request — a push commits instance-by-instance before replying).
   * @returns The correlated `WireMergeOutcome`; rejects with a `MergeIntentError` on a
   * correlated `merge_error`, or a plain `Error` on timeout/disconnect.
   * @example
   * ```
   * declare const session: WorldSession;
   * declare const childId: string;
   * await session.mergeIntent({ type: "merge_revert", request_id: crypto.randomUUID(), child_id: childId });
   * ```
   */
  mergeIntent(
    msg: Extract<
      ClientMsg,
      {
        /** Merge frame discriminant literal (`merge_pull`/`merge_push`/`merge_revert`). */
        type: "merge_pull" | "merge_push" | "merge_revert";
      }
    >,
    opts: WsTimeoutOptions = {},
  ): Promise<WireMergeOutcome> {
    if (!this.#ws) return Promise.reject(new Error("not connected"));
    return this.#ws.merge(msg, opts);
  }

  /** Draw one or more rows from a `table` document, posted as one chat message.
   * @param opts Draw options.
   * @returns Same silence-based resolution as `sendChatMessage`.
   * @example
   * ```
   * declare const session: WorldSession;
   * await session.drawTable({ tableId: "t1", channel: "general" });
   * ```
   */
  drawTable(opts: DrawTableOptions): Promise<void> {
    if (!this.#ws) return Promise.reject(new Error("not connected"));
    return this.#ws.drawTable(opts);
  }

  /** Subscribe to a SceneDerived channel. Returns a synchronous handle; the
   * underlying WS subscription is (re)established on every Welcome so derived state
   * survives a reconnect.
   * @param channel The SceneDerived channel name (e.g. a vision/lighting derivation).
   * @param onUpdate Called with each new frame on that channel.
   * @param opts Subscription options; see {@link SubscribeSceneOpts}.
   * @returns A synchronous handle; call `unsubscribe()` on it to stop receiving frames.
   * @example
   * ```
   * declare const session: WorldSession;
   * declare function render(payload: unknown): void;
   * const sub = session.subscribeScene("vision", (frame) => render(frame.payload));
   * sub.unsubscribe();
   * ```
   */
  subscribeScene(
    channel: string,
    onUpdate: (f: SceneFrame) => void,
    opts: SubscribeSceneOpts = {},
  ): SceneSubscription {
    const id = crypto.randomUUID();
    const rec = { channel, onUpdate, asUser: opts.asUser, level: opts.level, handle: null as SceneSubscription | null, gen: 0 };
    this.#sceneSubs.set(id, rec);
    this.#establishScene(id, rec);
    return {
      unsubscribe: () => {
        this.#sceneSubs.delete(id);
        rec.gen++; // invalidate any in-flight establish for this record
        rec.handle?.unsubscribe();
        rec.handle = null;
      },
    };
  }

  /** Establish (or re-establish) the live WS subscription for one `subscribeScene` record.
   * Called on first subscribe and again on every (re)connect Welcome. Generation-guarded:
   * a superseded attempt (unsubscribed, or a newer establish for the same record) self-disposes
   * its resolved handle instead of leaking a duplicate server subscription.
   * @param id The record's key in `#sceneSubs`.
   * @param rec The subscription record to (re)establish; see `SceneSubRecord`'s own field docs.
   * @example
   * ```
   * declare const id: string;
   * declare const rec: SceneSubRecord;
   * // called from subscribeScene and from #onWelcome; not part of the public API
   * this.#establishScene(id, rec);
   * ```
   */
  #establishScene(id: string, rec: SceneSubRecord): void {
    const ws = this.#ws;
    if (!ws) return;
    const gen = ++rec.gen; // this attempt's generation
    void ws
      .subscribeScene(rec.channel, rec.onUpdate, { asUser: rec.asUser, level: rec.level })
      .then((h) => {
        // Keep the handle only if this record is still active AND this is still the
        // latest establish attempt; a superseded attempt (re-establish on a new
        // Welcome, or an unsubscribe) self-disposes so no duplicate sub leaks.
        if (this.#sceneSubs.get(id) === rec && rec.gen === gen) rec.handle = h;
        else h.unsubscribe();
      })
      .catch(() => {
        // Dropped (e.g. disconnect during connect); re-established on the next Welcome.
      });
  }

  /** Open the WS connection to `worldId` and wire the broadcast-driven move/ping animation
   * listeners. Resolves once the connect ATTEMPT settles — `WsClient.open` catches a failed
   * `connect` and schedules a reconnect rather than rejecting, so resolution does NOT imply
   * the transport is up. It certainly does not imply the world is usable: the server's
   * Welcome, module activation, member fetch, and external-module loading all happen
   * asynchronously afterward inside `#onWelcome` and are NOT awaited by this call.
   *
   * Before opening the socket, fetches a current-state snapshot (`getWorldSnapshot`) and seeds
   * both `store` and `documents` from it, then pre-seeds the `WsClient`'s sequence watermark
   * (`WsClient.seedWatermark`) so a genuine cold start bootstraps from current state instead of
   * replaying the world's full history — the first `Welcome`'s existing gap-check only resyncs
   * whatever committed after the snapshot was read. A snapshot-fetch failure degrades
   * gracefully: it is logged and `enter` falls through to today's full-replay behavior (the
   * watermark stays unseeded, so the Welcome-triggered resync catches up from scratch), mirroring
   * `#loadExternalModules`'s own "a broken pipeline must never brick a world" pattern.
   * @param worldId The world to connect to.
   * @example
   * ```
   * declare const selfId: string;
   * declare const connect: Connect;
   * declare const layoutModule: Module;
   * declare const worldId: string;
   * const session = new WorldSession({ selfId, connect, modules: [layoutModule] });
   * await session.enter(worldId);
   * ```
   */
  async enter(worldId: string): Promise<void> {
    this.world = worldId;
    this.state = "connecting";
    let snapshotSeq: number | null = null;
    try {
      const snapshot = await getWorldSnapshot(worldId);
      this.store.seedDocuments(snapshot.documents);
      this.#optimistic.seedDocuments(snapshot.documents);
      snapshotSeq = snapshot.seq;
    } catch (e) {
      this.#logger.warn("world snapshot bootstrap failed; falling back to full replay", e);
    }
    this.#ws = new WsClient({
      world: worldId,
      connect: this.opts.connect,
      handlers: {
        // Feed both mirrors: the authoritative DocumentStore (exposed via
        // AppContext for document-reading panels) and the optimistic client
        // (base + pending view, given to modules as ctx.client).
        onCommand: (cmd) => {
          // Pre-scan cheaply: an ordinary token drag never touches a combat/combatant doc or
          // an embedded effect field, so it skips both the pre-image snapshot and the derive
          // call below entirely.
          const touches = commandTouchesCombat(cmd, this.store);
          const before = new Map<string, WireDocument | undefined>();
          if (touches) {
            for (const op of cmd.ops) {
              const id = op.op === "update" || op.op === "move" ? op.doc_id : op.doc.id;
              before.set(id, this.store.get(id));
            }
          }
          this.store.applyCommand(cmd);
          this.#optimistic.applyCommand(cmd);
          if (touches) {
            this.#combatEmitter.emit(deriveCombatHookEvents((id) => before.get(id), cmd, this.store));
          }
        },
        onReject: (id, reason, detail) => {
          this.#optimistic.reject(id);
          this.opts.onReject?.(reason, detail);
        },
        onWelcome: (w) => {
          void this.#onWelcome(w);
        },
        // After resync, authoritative state is current and the optimistic view has
        // rebased; replay any intents queued while offline so they converge.
        onResyncComplete: () => this.#flushOfflineQueue(),
        onError: (e) => this.#logger.error("world session ws error", e),
        onEvicted: () => this.opts.onEvicted?.(),
        onAssetChanged: (msg) => {
          // Bump the resolver first so a notified panel re-resolves the new URL.
          this.assets.onAssetChanged(msg);
          for (const cb of this.#assetListeners) cb(msg);
        },
        onScenePing: (msg) => {
          // Cross-scene guard: a scene_ping broadcasts room-wide and must render only for
          // recipients currently viewing that scene (a GM roaming scene B must not surface a
          // ping for scene A superimposed on B's grid, and vice versa). Mirrors the onMoveStream
          // scene filter above.
          if (msg.scene !== this.viewedSceneId) return;
          for (const cb of this.#pingListeners) cb(msg);
        },
        onEmote: (msg) => {
          // Cross-scene guard, same shape as the onScenePing filter above: an emote
          // broadcasts room-wide and must render only for recipients currently viewing
          // that scene.
          if (msg.scene !== this.viewedSceneId) return;
          for (const cb of this.#emoteListeners) cb(msg);
        },
        onAudioError: (reason) => this.opts.onAudioError?.(reason),
        onVfx: (msg) => {
          // Cross-scene guard, same shape as the onScenePing/onEmote filters above: a vfx
          // one-shot broadcasts room-wide and must render only for recipients currently
          // viewing that scene.
          if (msg.scene !== this.viewedSceneId) return;
          for (const cb of this.#vfxListeners) cb(msg);
        },
      },
    });
    // Pre-seed the watermark BEFORE start()/open() sends the first Hello — a call after open()
    // would move it backward instead (see WsClient.seedWatermark's own doc).
    if (snapshotSeq !== null) this.#ws.seedWatermark(snapshotSeq);
    // Broadcast-driven animation: drive all scene viewers (mover + observers) from the
    // MoveStream frame. serverNow() aligns startServerMs to local time for catch-up.
    // Coupling: sceneInteraction.animateSamples no-ops until Stage attaches the engine.
    const ws = this.#ws;
    // Unsub return discarded: the listener's lifetime equals this WsClient instance
    // (a fresh WsClient is created per enter() and discarded on leave()).
    this.#ws.onMoveStream((stream) => {
      // Cross-scene guard: a MoveStream broadcasts room-wide and is animated only if it targets the
      // scene THIS client is viewing (a GM roaming scene B must not animate scene A's move, and must
      // animate B's). `viewedSceneId` is the GM's local view when roaming, else the followed
      // `activeScene`. Fail-closed: a stream for any other scene is dropped (latent cross-scene
      // fog/animation leak, mirrors `RenderEngine.toVisibility`'s scene filter).
      if (stream.scene !== this.viewedSceneId) return;
      this.sceneInteraction.animateSamples(
        stream.tokenId,
        stream.samples,
        stream.durationMs,
        stream.startServerMs,
        () => ws.serverNow(),
        stream.moverVision,
        stream.moverLight,
      );
    });
    // Session-owned, not engine-owned: the resolved footprints feed the canvas, the hit-test and
    // the place tool, so they belong beside the document view every one of
    // those reads rather than inside the render engine one of them happens to live in. The first
    // attempt runs before the socket is up and is dropped; `#onWelcome` re-establishes it.
    this.#footprintsSub = this.subscribeScene("footprints", (f) => {
      this.#footprints = parseFootprints(f.payload);
    });
    // Same lifecycle as `#footprintsSub`: server-resolved per-recipient combat numbers, never
    // client-derived.
    this.#combatSub = this.subscribeScene("combat", (f) => {
      this.#combat.setResolved(parseCombats(f.payload, this.#logger));
    });
    // Same lifecycle as `#combatSub`: server-resolved spatial audio, never client-derived
    // geometry — `AudioEngine.applyAudibility` performs no falloff/occlusion math of its own.
    this.#audibilitySub = this.subscribeScene("audibility", (f) => {
      this.#audibilityPayload = parseAudibility(f.payload);
      this.#audioEngine.applyAudibility(sceneAudibility(this.#audibilityPayload, this.viewedSceneId));
    });
    // The audio-state singleton drives the device mixer: every authoritative change (this
    // world's own transport echoes included) reconciles the live TrackPlayer set. Plain
    // store-level subscription (this class is not a Svelte component), applied once
    // immediately for the state already in the snapshot.
    this.#audioUnsub = this.documents.subscribe(() => {
      const doc = this.documents.query(AUDIO_STATE_DOC_TYPE)[0];
      if (doc?.engine) this.#audioEngine.applyState(doc.engine as AudioStateEngine);
    });
    {
      const doc = this.documents.query(AUDIO_STATE_DOC_TYPE)[0];
      if (doc?.engine) this.#audioEngine.applyState(doc.engine as AudioStateEngine);
    }
    await this.#ws.start();
    this.state = "open";
  }

  /** Handle a `welcome` frame (first connect or a reconnect): sets `role`/`#worldGrants`/
   * `#requirements`, activates modules exactly once per session (see `#modulesAdded`/
   * `#activated`), loads external modules, fetches member usernames, reconciles contract
   * topology, re-establishes scene subscriptions, and (GM only) seeds the world's first scene.
   * Config singletons (`system-defaults` included) are server-seeded at world creation and
   * world join — this handler writes none of them. The member fetch has its own inner
   * try/catch: a failure there is logged as a
   * warning and does not block the remaining steps. Module activation's inner catch instead
   * reverts `#activated` (so the next Welcome retries it) and logs at the point of failure —
   * it does not rethrow, so every later step in this call (member fetch, `reconcileTopology`,
   * scene resubscription, the GM first-scene seed) still runs even
   * when activation fails; the whole method is also wrapped in one outer try/catch that logs
   * and swallows any other failure, so this promise never rejects.
   * @param w The Welcome frame.
   * @example
   * ```
   * declare const w: WireWelcome;
   * // wired as handlers.onWelcome in enter(); not part of the public API
   * void this.#onWelcome(w);
   * ```
   */
  async #onWelcome(w: WireWelcome): Promise<void> {
    try {
      this.role = w.user_role;
      this.#worldGrants = w.world_default_grants;
      this.#requirements = w.capability_requirements;
      this.#roleCaps = w.role_capabilities;
      this.#serverVersion = w.server_version;
      // Snapshot BEFORE any await below: a scene subscription added while this
      // Welcome's async chain is still in flight (module activation / external-module
      // load / member fetch) already self-establishes via `subscribeScene`'s own
      // `#establishScene` call — reconciling it again here too would double-send
      // `scene_subscribe` for the very sub this Welcome never actually dropped.
      const subsAtWelcome = [...this.#sceneSubs];
      // Activate modules BEFORE any await below (the member fetch) so the
      // layout module contributes Layout into the `root` surface the host renders
      // — the table chrome paints immediately on mount, never a blank frame during
      // the member-fetch round-trip. `#modulesAdded` and `#activated` are both set
      // synchronously BEFORE the `activate()` await: Welcome frames delivered back
      // to back in the same tick (e.g. a `Connect` that replays a burst on
      // reconnect) invoke `#onWelcome` re-entrantly while the first call is still
      // suspended at that await, and only a synchronous guard closes that window —
      // setting `#activated` in a `.then()`/after-await would let the second
      // in-flight call see it still `false` and double-activate every module. A
      // GENUINELY failed activation (thrown, e.g. a contract cycle) reverts
      // `#activated` to `false` in the catch below so a later, sequential Welcome
      // retries it — it is not cached for the session's life.
      if (!this.#activated) {
        if (!this.#modulesAdded) {
          this.#modulesAdded = true;
          for (const m of this.opts.modules) this.#modules.add(m);
        }
        this.#activated = true;
        try {
          await this.#modules.activate();
          await this.#loadExternalModules(w.world, w.server_version);
        } catch (e) {
          this.#activated = false;
          this.#logger.error(
            "module activation failed; Surfaces degrade until a later Welcome retries",
            e,
          );
        }
      }
      // Fetch member usernames: every role needs these to resolve chat author
      // names and whisper recipient labels; the GM additionally uses them for
      // see-as labels. Best-effort: a failure leaves those UIs on short-id
      // fallback. The members SvelteMap is mutated in place, so consumers
      // (already rendered after activation) populate reactively when this
      // resolves.
      try {
        const list = await listWorldMembers(w.world);
        // Mutate in place (not reassign) so the AppContext-captured reference
        // stays valid; reconnect re-populates the same Map.
        this.members.clear();
        for (const m of list) this.members.set(m.user, m.username);
      } catch (e) {
        this.#logger.warn("member list fetch failed", e);
      }
      reconcileTopology(this.#modules.declarations(), w.contract_declarations, this.#logger);
      // Scene subscriptions are dropped by the WS on disconnect; re-establish each
      // on every (re)connect so derived state (vision) survives a reconnect. No-op
      // on the first Welcome (none registered until the render engine subscribes).
      // Iterates the PRE-await snapshot (`subsAtWelcome`), not the live map, so a sub
      // added mid-flight (see snapshot comment above) is left to its own establish.
      for (const [id, rec] of subsAtWelcome) {
        // Liveness check: `subsAtWelcome` holds `[id, rec]` by reference, so a caller
        // that unsubscribes during this Welcome's (now-widened) await window has
        // already removed `id` from the live `#sceneSubs` map. Skip a torn-down entry
        // instead of resurrecting it with a spurious `scene_subscribe`.
        if (this.#sceneSubs.get(id) !== rec) continue;
        // Tear down a live handle from a prior connect before re-subscribing; the
        // gen bump inside #establishScene invalidates any still-in-flight attempt,
        // so a flapping reconnect can't leak a duplicate server subscription.
        rec.handle?.unsubscribe();
        rec.handle = null;
        this.#establishScene(id, rec);
      }
      // Ensure an active scene exists so the place tool has a parent to
      // attach tokens to. GM-only (players can't author the world's first scene);
      // guard on the optimistic view (includes the pending create) so a reconnect
      // Welcome — or a scene from another GM — does not double-create. The rare
      // multi-GM simultaneous-first-entry double-create is accepted.
      if (this.role === "gm" && this.world && this.#optimistic.query("scene").length === 0) {
        this.dispatchIntent([{ op: "create", doc: buildSceneDoc(this.world) }]);
      }
    } catch (e) {
      this.#logger.error("world session welcome handling failed", e);
    }
  }

  /** Fetch the world's enabled installed-module set + their (manifest,
   * entry_url) pairs and load them through the shared, per-module-contained
   * loader. Runs exactly once per WorldSession (called only inside
   * the `#activated` guard, after a successful `activate()`) — a reconnect within one session
   * never re-runs this bootstrap load; `reconcileInstalledModules` is the separate, explicitly
   * triggered path that hot-reloads a running session's external-module set. A discovery-level
   * failure (network, malformed response) degrades to a logged warning; the session still enters
   * the world with only its first-party modules active — a broken pipeline must never brick
   * a world.
   * @param world The world id to load enabled external modules for.
   * @param serverVersion The connected server's version, passed through to `loadModules`'
   * engine-compat gate.
   * @example
   * ```
   * declare const w: WireWelcome;
   * // called from #onWelcome after a successful module activation; not part of the public API
   * await this.#loadExternalModules(w.world, w.server_version);
   * ```
   */
  async #loadExternalModules(world: string, serverVersion: string): Promise<void> {
    try {
      const [enabledEntries, installed] = await Promise.all([
        getEnabledModules(world),
        listInstalledModules(),
      ]);
      const enabledIds = enabledEntries.map((e) => e.id);
      const resolved = WorldSession.#buildEntries(enabledIds, installed, this.#logger);
      if (resolved.length === 0) return;
      const result = await loadModules({
        entries: resolved.map(({ manifest, entry }) => ({ manifest, entry })),
        importFn: this.opts.importModule ?? ((url) => import(/* @vite-ignore */ url)),
        registry: this.#modules,
        shadowcatVersion: serverVersion,
      });
      for (const f of result.failed) {
        this.#logger.warn(`external module ${f.id} (${f.entry}) failed to load: ${f.error}`);
      }
      WorldSession.#recordLoaded(this.#externalModuleIds, resolved, result.loaded);
      if (result.loaded.length > 0) await this.#modules.activate();
      // Styles follow activation, never precede it, and only a module that
      // actually activated gets a link: a per-module activation failure logs
      // and skips without rejecting (see `ModuleRegistry.activate`), so
      // `result.loaded` can name a module that never activated.
      WorldSession.#applyModuleStyles(
        this.#moduleStyleLinks,
        resolved,
        result.loaded.filter((id) =>
          this.#modules.list().some((m) => m.id === id && m.active),
        ),
      );
    } catch (e) {
      this.#logger.warn("external module discovery failed", e);
    }
  }

  /** Resolves an enabled-id list against the installed catalog, keyed on the canonical install
   * folder id (`info.id`) — matching the server's own enabled-set key space, NOT `manifest.id`,
   * which is an opaque, author-declared value the folder id may legitimately differ from (or
   * collide with another module's). An enabled id absent from `installed` is logged and skipped.
   * Shared by `#loadExternalModules` and `reconcileInstalledModules` so the lookup logic can't
   * drift between the two. Returns the folder id alongside each entry (not just the bare
   * `ModuleEntry` `loadModules` consumes) because `#recordLoaded` needs it to map a load result's
   * manifest id back to the folder id `#externalModuleIds` is keyed on.
   * @param ids The enabled module (folder) ids to resolve.
   * @param installed The full installed-module catalog to resolve `ids` against.
   * @param logger Diagnostics sink for a skipped (not-installed) id.
   * @returns The resolved folder id + `(manifest, entry)` triples, in `ids` order.
   * @example
   * ```
   * declare const ids: string[];
   * declare const installed: InstalledModuleInfo[];
   * declare const logger: Logger;
   * // called from #loadExternalModules and reconcileInstalledModules; not part of the public API
   * WorldSession.#buildEntries(ids, installed, logger);
   * ```
   */
  static #buildEntries(
    ids: string[],
    installed: InstalledModuleInfo[],
    logger: Logger,
  ): ResolvedModuleEntry[] {
    const byId = new Map<string, InstalledModuleInfo>();
    for (const info of installed) byId.set(info.id, info);
    const resolved: ResolvedModuleEntry[] = [];
    for (const id of ids) {
      const info = byId.get(id);
      if (!info) {
        logger.warn(`enabled module ${id} is not installed; skipping`);
        continue;
      }
      resolved.push({
        folderId: id,
        manifest: info.manifest as ModuleManifest,
        entry: info.entry_url,
      });
    }
    return resolved;
  }

  /** Records every successfully loaded module from a `loadModules` result into `into`, mapping
   * each loaded manifest id back to the folder id `#buildEntries` resolved it from — `loaded`
   * carries only manifest ids (`loadModules`' own `ModuleLoadResult.loaded` doc), which is not
   * the key space `#externalModuleIds` is diffed against (see that field's own doc for why both
   * id spaces are load-bearing).
   * @param into The map to record into (mutated in place); folder id → manifest id.
   * @param resolved The same triples passed to `loadModules` (via `#buildEntries`), used to
   * recover each loaded manifest id's folder id.
   * @param loaded The manifest ids `loadModules` reports as successfully loaded.
   * @example
   * ```
   * declare const into: Map<string, string>;
   * declare const resolved: ResolvedModuleEntry[];
   * declare const loaded: string[];
   * // called from #loadExternalModules and reconcileInstalledModules; not part of the public API
   * WorldSession.#recordLoaded(into, resolved, loaded);
   * ```
   */
  static #recordLoaded(
    into: Map<string, string>,
    resolved: ResolvedModuleEntry[],
    loaded: string[],
  ): void {
    const folderIdByManifestId = new Map(resolved.map((r) => [r.manifest.id, r.folderId]));
    for (const manifestId of loaded) {
      const folderId = folderIdByManifestId.get(manifestId);
      if (folderId !== undefined) into.set(folderId, manifestId);
    }
  }

  /** Injects the declared stylesheet (`ModuleManifest.style`) of every newly
   * loaded module as a `<link>` into the document head, resolved against the
   * module's own entry URL (so the href stays inside the module's served
   * folder). Follows `#recordLoaded`'s manifest-id↔folder-id mapping; per-
   * module contained like the loader itself — a module whose link cannot be
   * created never affects the others.
   * @param into The link map to record into (mutated in place), manifest id →
   * link element.
   * @param resolved The same triples passed to `loadModules`.
   * @param loaded The manifest ids `loadModules` reports as successfully
   * loaded.
   * @example
   * ```
   * declare const into: Map<string, HTMLLinkElement>;
   * declare const resolved: ResolvedModuleEntry[];
   * declare const loaded: string[];
   * // called from #loadExternalModules and reconcileInstalledModules; not part of the public API
   * WorldSession.#applyModuleStyles(into, resolved, loaded);
   * ```
   */
  static #applyModuleStyles(
    into: Map<string, HTMLLinkElement>,
    resolved: ResolvedModuleEntry[],
    loaded: string[],
  ): void {
    if (typeof document === "undefined") return;
    const byManifestId = new Map(resolved.map((r) => [r.manifest.id, r]));
    for (const manifestId of loaded) {
      const style = byManifestId.get(manifestId)?.manifest.style;
      const entry = byManifestId.get(manifestId)?.entry;
      if (!style || !entry || into.has(manifestId)) continue;
      const link = document.createElement("link");
      link.rel = "stylesheet";
      // The manifest schema already rejects absolute/traversal paths; the
      // entry URL's own directory is the module's served root.
      link.href = new URL(style, new URL(entry, document.baseURI)).pathname;
      link.dataset.shadowcatModuleStyle = manifestId;
      document.head.appendChild(link);
      into.set(manifestId, link);
    }
  }

  /** Removes one module's injected stylesheet link, if any.
   * @param manifestId The unloaded module's manifest id.
   * @example
   * ```
   * // private method; not part of the public API — invoked from
   * // reconcileInstalledModules' unload loop and leave()
   * declare const session: WorldSession;
   * session.leave();
   * ```
   */
  #removeModuleStyle(manifestId: string): void {
    this.#moduleStyleLinks.get(manifestId)?.remove();
    this.#moduleStyleLinks.delete(manifestId);
  }

  /** Re-fetches this world's enabled external-module set and reconciles the running session
   * against it: unloads (cascade) any currently-loaded external module no longer enabled, and
   * loads + activates any newly-enabled one. First-party modules (`opts.modules`) are never
   * touched — this is deliberately scoped to external/community modules only, mirroring the
   * `ModuleManager.svelte` GM UI's own scope. Per-entry contained: one module's load/unload
   * failure is logged and does not abort the others. A no-op if called before `#onWelcome` has
   * ever run (`world`/`#serverVersion` unset).
   * @returns Resolves once every diffed module has been attempted; never rejects.
   * @example
   * ```
   * declare const session: WorldSession;
   * await session.reconcileInstalledModules();
   * ```
   */
  async reconcileInstalledModules(): Promise<void> {
    if (!this.world || this.#serverVersion === undefined) return;
    try {
      const [enabledEntries, installed] = await Promise.all([
        getEnabledModules(this.world),
        listInstalledModules(),
      ]);
      const enabledIds = enabledEntries.map((e) => e.id);
      const enabledSet = new Set(enabledIds);
      const toUnload = [...this.#externalModuleIds].filter(([folderId]) => !enabledSet.has(folderId));
      for (const [folderId, manifestId] of toUnload) {
        try {
          await this.#modules.unload(manifestId, { cascade: true });
          this.#externalModuleIds.delete(folderId);
          this.#removeModuleStyle(manifestId);
        } catch (e) {
          this.#logger.warn(`external module ${manifestId} failed to unload during reconcile`, e);
        }
      }
      const toLoadIds = enabledIds.filter((id) => !this.#externalModuleIds.has(id));
      if (toLoadIds.length === 0) return;
      const resolved = WorldSession.#buildEntries(toLoadIds, installed, this.#logger);
      if (resolved.length === 0) return;
      const result = await loadModules({
        entries: resolved.map(({ manifest, entry }) => ({ manifest, entry })),
        importFn: this.opts.importModule ?? ((url) => import(/* @vite-ignore */ url)),
        registry: this.#modules,
        shadowcatVersion: this.#serverVersion,
      });
      for (const f of result.failed) {
        this.#logger.warn(`external module ${f.id} (${f.entry}) failed to load during reconcile: ${f.error}`);
      }
      WorldSession.#recordLoaded(this.#externalModuleIds, resolved, result.loaded);
      if (result.loaded.length > 0) await this.#modules.activate();
      // Styles follow activation, never precede it, and only a module that
      // actually activated gets a link: a per-module activation failure logs
      // and skips without rejecting (see `ModuleRegistry.activate`), so
      // `result.loaded` can name a module that never activated.
      WorldSession.#applyModuleStyles(
        this.#moduleStyleLinks,
        resolved,
        result.loaded.filter((id) =>
          this.#modules.list().some((m) => m.id === id && m.active),
        ),
      );
    } catch (e) {
      this.#logger.warn("external module reconcile failed", e);
    }
  }

  /** Tear down the current world connection: stops and drops the `WsClient`, resets
   * `state`/`role`/`world`/`#gmViewedScene`. Does not clear `store`/`documents`, module
   * registrations, or the `#activated`/`#modulesAdded` latches — the shell's own usage
   * (`App`'s `leaveWorld`) discards this instance and constructs a fresh
   * `WorldSession` for the next `enter()` rather than reusing this one.
   * @example
   * ```
   * declare const session: WorldSession;
   * session.leave();
   * ```
   */
  leave(): void {
    this.#footprintsSub?.unsubscribe();
    this.#footprintsSub = null;
    this.#footprints = EMPTY_FOOTPRINTS;
    this.#combatSub?.unsubscribe();
    this.#combatSub = null;
    this.#combat.setResolved(EMPTY_COMBATS);
    this.#audibilitySub?.unsubscribe();
    this.#audibilitySub = null;
    this.#audibilityPayload = EMPTY_AUDIBILITY;
    this.#audioUnsub?.();
    this.#audioUnsub = null;
    this.#audioEngine.dispose();
    for (const manifestId of [...this.#moduleStyleLinks.keys()]) {
      this.#removeModuleStyle(manifestId);
    }
    this.#ws?.stop();
    this.#ws = null;
    this.state = "closed";
    this.role = null;
    this.world = null;
    this.#gmViewedScene = null;
  }
}
