<script lang="ts">
  import { webSocketConnect } from "@shadowcat/core";
  import { plainHttpHint } from "./lib/plainHttpHint";
  import { TlsStatusLoader } from "./lib/tlsStatusLoader.svelte";
  import { notifications, t } from "@shadowcat/ui-kit";
  import { Entry } from "@shadowcat/module-entry";
  import { getMe, getTlsStatus, listWorlds, withRetry, type Me } from "./lib/api";
  import {
    loadSessionState,
    setLastWorld,
    flushOnUnload,
    pruneStaleWorlds,
  } from "./lib/sessionState.svelte";
  import { currentRoute, navigate } from "./lib/route.svelte";
  import { resolveBootWorld } from "./lib/bootResolution";
  import { coreUi } from "@shadowcat/module-core-ui";
  import { panels } from "@shadowcat/module-panels";
  import { topBar } from "@shadowcat/module-topbar";
  import { statusBar } from "@shadowcat/module-statusbar";
  import { stage } from "@shadowcat/module-stage";
  import { settings } from "@shadowcat/module-settings";
  import { assetBrowser } from "@shadowcat/module-asset-browser";
  import { actors } from "@shadowcat/module-actors";
  import { factions } from "@shadowcat/module-factions";
  import { conditions } from "@shadowcat/module-conditions";
  import { combatTracker } from "@shadowcat/module-combat-tracker";
  import { gameSettings } from "@shadowcat/module-game-settings";
  import { sceneBrowser } from "@shadowcat/module-scene-browser";
  import { sceneTools } from "@shadowcat/module-scene-tools";
  import { chat } from "@shadowcat/module-chat";
  import { chatComposer } from "@shadowcat/module-chat-composer";
  import { chatCard } from "@shadowcat/module-chat-card";
  import { notes } from "@shadowcat/module-notes";
  import { tables } from "@shadowcat/module-tables";
  import { sheetFallback } from "@shadowcat/module-sheet-fallback";
  import { sheetActor } from "@shadowcat/module-sheet-actor";
  import { sheetItem } from "@shadowcat/module-sheet-item";
  import { sheetNote } from "@shadowcat/module-sheet-note";
  import { sheetTable } from "@shadowcat/module-sheet-table";
  import { dice3d } from "@shadowcat/module-dice-3d";
  import { audio } from "@shadowcat/module-audio";
  import { sheetPlaylist } from "@shadowcat/module-sheet-playlist";
  import { vfx } from "@shadowcat/module-vfx";
  import { ducking } from "@shadowcat/module-ducking";
  import { untrack } from "svelte";
  import { WorldSession } from "./lib/worldSession.svelte";
  import Table from "./lib/Table.svelte";

  /** Overall cap on `boot()`'s wall-clock time before it gives up and falls back to the
   * login/worlds route, even if a `withRetry` chain is still in flight. Comfortably less than half
   * of the ~141s worst case for three full sequential `withRetry` cycles (see `boot()`'s own doc),
   * while still covering a SINGLE `withRetry` call's own worst case (~47s: 3 attempts at
   * `FETCH_TIMEOUT_MS` plus inter-attempt delays) with margin, so a user on a slow-but-live
   * connection is not cut off mid-way through just the first network call. */
  const BOOT_DEADLINE_MS = 60_000;

  /** How long `boot()` waits before switching its loading message from "Loading…" to "Still
   * trying…" — shorter than a single fetch attempt's own `FETCH_TIMEOUT_MS` (15s), so the message
   * changes while the very first attempt of the very first `withRetry` call may still be in
   * flight on a slow-but-live connection, giving feedback before any retry has even happened. */
  const STILL_TRYING_AFTER_MS = 8_000;

  let me = $state<Me | null>(null);
  let booted = $state(false);
  let bootStillTrying = $state(false);
  let session = $state<WorldSession | null>(null);
  /** The server's published TLS status (`tls.status`): `null` until read, or when the read
   * failed, in which case the TLS-dependent hints stay silent rather than guessing. */
  const tls = new TlsStatusLoader(getTlsStatus);
  void tls.ensure();

  /** After a run of first-connect WebSocket failures, offers the plain-HTTP address when
   * `plainHttpHint` finds the failure looks like the browser refusing `wss:` (see its doc for the
   * discriminator).
   * @param failures The consecutive failure count.
   * @example
   * ```
   * await hintPlainHttpIfWssRefused(3);
   * ```
   */
  async function hintPlainHttpIfWssRefused(failures: number): Promise<void> {
    const url = await plainHttpHint(failures, {
      location,
      currentTls: () => tls.status,
      loadTls: () => tls.ensure(),
      fetchMe: () => fetch("/api/me", { credentials: "same-origin" }),
    });
    if (url !== null) notifications.push("warning", t("ws.plainHttpHint", { url }));
  }

  /** Resolves the app's initial route on load: fetches identity, applies the
   * saved locale, and — only when the URL or a persisted `lastWorld` could
   * resolve to a world — fetches the worlds list and re-enters via
   * `resolveBootWorld`. Falls back to the login/worlds route on any failure
   * (a hung/failing backend must not wedge the SPA on "Loading…" forever)
   * and always sets `booted = true` on exit. Bounded overall by
   * `BOOT_DEADLINE_MS`: past that deadline it abandons any in-flight fetch
   * and falls back to `login` regardless of `me`, and further navigation/session-entry from an
   * abandoned fetch is a no-op (checked after every await via the closure-local `abandoned` flag,
   * which also gates the `me` assignment itself) — a side effect embedded inside an awaited call
   * (e.g. `loadSessionState`'s locale write) still applies once that call resolves, since the
   * underlying fetch itself is not cancelled. Switches the caller's rendered message
   * from "Loading…" to "Still trying…" via `bootStillTrying` after
   * `STILL_TRYING_AFTER_MS`.
   * @returns Resolves once the initial route has been decided (or the deadline fires); never
   *   rejects. Which route a failure degrades to depends on where it happened: a
   *   `getMe`/`loadSessionState` failure (outer catch) goes to `login`, a `listWorlds` failure
   *   once `me` is known falls through to `navigate({ name: me ? "worlds" : "login" })` — i.e.
   *   `worlds`, since `me` is truthy on that path — and the overall deadline always goes to
   *   `login`.
   * @example
   * ```
   * boot();
   * ```
   */
  async function boot() {
    let abandoned = false;
    const deadlineTimer = setTimeout(() => {
      abandoned = true;
      navigate({ name: "login" });
      booted = true;
    }, BOOT_DEADLINE_MS);
    const stillTryingTimer = setTimeout(() => {
      bootStillTrying = true;
    }, STILL_TRYING_AFTER_MS);
    try {
      const fetchedMe = await withRetry(() => getMe());
      if (abandoned) return;
      me = fetchedMe;
      if (me) {
        const ui = await withRetry(() => loadSessionState()); // applies the saved locale
        if (abandoned) return;
        const last = ui.global.lastWorld;
        // A world route in the URL always wins over lastWorld (resolveBootWorld) —
        // only call /api/worlds when either could resolve to an entry.
        const route = currentRoute();
        if (route.name === "world" || last) {
          // A transient /api/worlds failure here degrades to entry, not a hard error.
          try {
            const worlds = await withRetry(() => listWorlds());
            if (abandoned) return;
            pruneStaleWorlds(worlds.map((w) => w.id));
            const resolved = resolveBootWorld(currentRoute(), last, worlds);
            if (resolved.enterWorldId) {
              // Reload returns you to the URL's/last world; the session follows the route.
              navigate({ name: "world", id: resolved.enterWorldId });
              return;
            }
            if (resolved.clearLastWorld) setLastWorld(null); // stale (deleted / revoked) — clear it
          } catch {
            // fall through to entry
          }
        }
      }
      if (abandoned) return;
      // Seeds the URL hash only; <Entry> derives the actual pre-world step (setup/
      // login/world-select) internally — every pre-world route renders <Entry>.
      navigate({ name: me ? "worlds" : "login" });
    } catch {
      // A transient backend failure must not wedge the SPA on "Loading…".
      if (!abandoned) navigate({ name: "login" });
    } finally {
      clearTimeout(deadlineTimer);
      clearTimeout(stillTryingTimer);
      booted = true;
      bootStillTrying = false;
    }
  }
  boot();

  // Best-effort persist of a still-pending ui_state change when the tab is hidden
  // or unloaded (the debounce's trailing write would otherwise not fire).
  if (typeof window !== "undefined") {
    window.addEventListener("pagehide", flushOnUnload);
    window.addEventListener("visibilitychange", () => {
      if (document.visibilityState === "hidden") flushOnUnload();
    });
  }

  /** Called by `<Entry>` once its own login flow succeeds. Fetches identity
   * and applies the saved session state (locale) so both are in hand before
   * `<Entry>` advances to world-select.
   * @returns Whether identity was fetched successfully. `<Entry>` returns to
   *   login when this resolves `false`.
   * @example
   * ```
   * const ok = await onAuthenticated();
   * ```
   */
  async function onAuthenticated(): Promise<boolean> {
    try {
      me = await getMe();
      await loadSessionState();
    } catch {
      me = null;
    }
    return me !== null;
  }

  /** Opens the WebSocket session for `worldId`. Not reactive: the caller (the session effect
   * below) decides when a session exists.
   * @param worldId - The world to enter.
   * @param selfId - The signed-in user's id.
   * @returns The new session, already entering `worldId`.
   * @example
   * ```
   * const s = openSession("w1", "u1");
   * ```
   */
  function openSession(worldId: string, selfId: string): WorldSession {
    const wsUrl =
      (location.protocol === "https:" ? "wss:" : "ws:") +
      "//" + location.host + "/ws?world=" + worldId;
    const s = new WorldSession({
      selfId,
      connect: webSocketConnect(wsUrl),
      modules: [panels, coreUi, topBar, statusBar, stage, settings, gameSettings, sceneBrowser, assetBrowser, actors, factions, conditions, combatTracker, sceneTools, chat, chatComposer, chatCard, notes, tables, sheetFallback, sheetActor, sheetItem, sheetNote, sheetTable, dice3d, audio, sheetPlaylist, vfx, ducking],
      onEvicted: () => navigate({ name: "worlds" }),
      // A refused join (the world is gone, or this user is not a member) is terminal: say why,
      // then leave the world route so the session effect tears the session down.
      onJoinRefused: (code) => {
        notifications.push("warning", t(`session.joinRefused.${code}`));
        navigate({ name: "worlds" });
      },
      onReject: (reason, detail) =>
        notifications.push(
          "warning",
          detail ? `${t(`intent.rejected.${reason}`)} ${detail}` : t(`intent.rejected.${reason}`),
        ),
      // The audio refusal reason is already player-presentable free text produced server-side
      // (unlike `RejectReason`'s fixed key set) — pushed raw, never through `t()`.
      onAudioError: (reason) => notifications.push("warning", reason),
      // A browser can render an https: page after a certificate exception and still refuse
      // `wss:`; the same server answers plain HTTP on the same port, so name that address.
      onConnectFailure: (n) => void hintPlainHttpIfWssRefused(n),
      // A welcome (first connect or reconnect) re-reads the TLS status when every earlier read failed.
      onWelcome: () => void tls.ensure(),
    });
    void s.enter(worldId);
    return s;
  }

  /** Enters a world by navigating to its route; the session effect below opens the session.
   * @param worldId - The world to enter.
   * @example
   * ```
   * enterWorld("w1");
   * ```
   */
  function enterWorld(worldId: string) {
    navigate({ name: "world", id: worldId });
  }

  /** Leaves the current world by navigating to the worlds list; the session effect below tears
   * the session down.
   * @example
   * ```
   * leaveWorld();
   * ```
   */
  function leaveWorld() {
    navigate({ name: "worlds" });
  }

  const route = $derived(currentRoute());
  /** The world the route names, `null` for every other route. A string, so the session effect
   * below re-runs only when the world changes: `navigate` and the `hashchange` it triggers each
   * assign an equal-but-distinct route object. */
  const routeWorldId = $derived(route.name === "world" ? route.id : null);

  // The route is the single source of truth for the session: a session exists exactly while the
  // route names a world, the app has booted and an identity is known, and it is the session OF
  // THAT WORLD. Every way the route changes (entry screen, eviction, the back/forward buttons, an
  // edited URL, boot's resolved world) reaches the same owner here, so no path can leave a
  // session running for a route that no longer names its world. The effect depends on the world
  // id only (and on `booted`/`me`), never on the session it creates; its cleanup tears the
  // session down and clears `lastWorld`, and the next run sets it again for the new world.
  $effect(() => {
    const worldId = routeWorldId;
    if (!booted || !me || worldId === null) return;
    const selfId = me.id;
    // Everything below reads or writes reactive state other than the three dependencies above
    // (`session`, the persisted UI state); none of it may re-run this effect.
    const s = untrack(() => {
      const opened = openSession(worldId, selfId);
      session = opened;
      setLastWorld(worldId);
      return opened;
    });
    return () => {
      s.leave();
      if (session === s) session = null;
      setLastWorld(null);
    };
  });
</script>

{#if !booted}
  <p class="connecting">{bootStillTrying ? "Still trying…" : "Loading…"}</p>
{:else if route.name === "world" && me && session?.role && session?.world}
  {#key session}
    <Table {session} {leaveWorld} tls={tls.status} serverRole={me?.server_role === "admin" ? "admin" : "user"} />
  {/key}
{:else if route.name === "world" && me}
  <p class="connecting">Connecting…</p>
{:else}
  <Entry {onAuthenticated} onEnterWorld={enterWorld} />
{/if}
