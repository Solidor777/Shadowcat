# First-party modules

Shadowcat's in-game UI is **UI-as-modules**: every element — the layout grid,
the top bar, the stage canvas, chat, sheets — is a module contributing
components into contracts. Modules never import each other; they communicate
only through seams (`provides`/`requires` contract declarations, the
contribution registry, `<Surface>` hosts, AppContext, and the render-layer
API). Any of them can be replaced by a community module that claims the same
contract.

Community modules follow the same model — see
[Creating a module](/guides/creating-a-module).

| Module | Purpose |
|---|---|
| [entry](/modules/entry) | Pre-world experience: login, setup, world select |
| [core-ui](/modules/core-ui) | The responsive region grid every other element renders into |
| [topbar](/modules/topbar) | Top bar: launcher menu, presence |
| [statusbar](/modules/statusbar) | Status bar + minimized-panel chip strip |
| [panels](/modules/panels) | The dockable panel manager (`shadowcat.panel` host) |
| [settings](/modules/settings) | Settings panel: users, invites, modules, locale, session |
| [game-settings](/modules/game-settings) | GM game configuration panel |
| [assets](/modules/assets) | Asset upload/browse panel |
| [scene-browser](/modules/scene-browser) | GM scene list/activate/roam panel |
| [sheet-fallback](/modules/sheet-fallback) | The always-available generic document sheet |
| [stage](/modules/stage) | The PixiJS scene canvas |
| [scene-tools](/modules/scene-tools) | Scene tool rail: place/select/move/draw/measure/ping/wall/region |
| [actors](/modules/actors) | Actor browser panel (live search, open sheet) |
| [factions](/modules/factions) | Faction registry panel |
| [conditions](/modules/conditions) | Condition registry panel |
| [chat](/modules/chat) | Chat panel: channels, messages, rolls |
| [chat-composer](/modules/chat-composer) | Message composer contribution |
| [chat-card](/modules/chat-card) | Message rendering contribution |
| [sheet-actor](/modules/sheet-actor) | Generic actor sheet (priority 0) |
| [sheet-item](/modules/sheet-item) | Generic item sheet (priority 0) |

A nested `src/modules/nightfox/` checkout, when present, is the external
Nightfox system repo in its dev position — it documents itself in its own
repository.
