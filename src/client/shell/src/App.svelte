<script lang="ts">
  import { webSocketConnect } from "@shadowcat/core";
  import { Entry } from "@shadowcat/module-entry";
  import { getMe, listWorlds, withRetry, type Me } from "./lib/api";
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
  import { gameSettings } from "@shadowcat/module-game-settings";
  import { sceneBrowser } from "@shadowcat/module-scene-browser";
  import { sceneTools } from "@shadowcat/module-scene-tools";
  import { chat } from "@shadowcat/module-chat";
  import { chatComposer } from "@shadowcat/module-chat-composer";
  import { chatCard } from "@shadowcat/module-chat-card";
  import { sheetFallback } from "@shadowcat/module-sheet-fallback";
  import { sheetActor } from "@shadowcat/module-sheet-actor";
  import { sheetItem } from "@shadowcat/module-sheet-item";
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
              enterWorld(resolved.enterWorldId); // reload returns you to the URL's/last world
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

  /** Enters a world: opens its WebSocket session, persists it as
   * `lastWorld`, and navigates to `#/world/<id>`. A no-op if identity has
   * not been established yet (`me` is `null`).
   * @param worldId - The world to enter.
   * @example
   * ```
   * enterWorld("w1");
   * ```
   */
  function enterWorld(worldId: string) {
    if (!me) return;
    const wsUrl =
      (location.protocol === "https:" ? "wss:" : "ws:") +
      "//" + location.host + "/ws?world=" + worldId;
    const s = new WorldSession({ selfId: me.id, connect: webSocketConnect(wsUrl), modules: [panels, coreUi, topBar, statusBar, stage, settings, gameSettings, sceneBrowser, assetBrowser, actors, factions, conditions, sceneTools, chat, chatComposer, chatCard, sheetFallback, sheetActor, sheetItem], onEvicted: () => leaveWorld() });
    session = s;
    void s.enter(worldId);
    setLastWorld(worldId);
    navigate({ name: "world", id: worldId });
  }

  /** Leaves the current world: tears down the session, clears `lastWorld`,
   * and navigates back to the worlds list.
   * @example
   * ```
   * leaveWorld();
   * ```
   */
  function leaveWorld() {
    session?.leave();
    session = null;
    setLastWorld(null);
    navigate({ name: "worlds" });
  }

  const route = $derived(currentRoute());
</script>

{#if !booted}
  <p class="connecting">{bootStillTrying ? "Still trying…" : "Loading…"}</p>
{:else if route.name === "world" && session?.role && session?.world}
  <Table {session} {leaveWorld} serverRole={me?.server_role === "admin" ? "admin" : "user"} />
{:else if route.name === "world"}
  <p class="connecting">Connecting…</p>
{:else}
  <Entry {onAuthenticated} onEnterWorld={enterWorld} />
{/if}
