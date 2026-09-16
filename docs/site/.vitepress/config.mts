import { defineConfig } from "vitepress";

// Portal for the assembled docs site. The generated references (api/ts, api/rust)
// are copied in AFTER this build by scripts/assemble-docs.mjs, so /api/ links are
// outside VitePress's dead-link graph and are validated by the assembly script's
// own link check instead.
export default defineConfig({
  title: "Shadowcat",
  description: "Self-hostable, fully moddable virtual tabletop — documentation",
  srcDir: ".",
  ignoreDeadLinks: [/^\.?\.?\/?api\//],
  themeConfig: {
    nav: [
      { text: "Guides", link: "/guides/hosting" },
      { text: "Modules", link: "/modules/" },
      { text: "Protocol", link: "/protocol" },
      { text: "TS API", link: "/api/ts/", target: "_self" },
      { text: "Rust API", link: "/api/rust/shadowcat/", target: "_self" },
    ],
    sidebar: {
      "/guides/": [
        {
          text: "Guides",
          items: [
            { text: "Hosting a server", link: "/guides/hosting" },
            { text: "Creating a module", link: "/guides/creating-a-module" },
            { text: "Creating a system", link: "/guides/creating-a-system" },
          ],
        },
      ],
      "/modules/": [
        {
          text: "Shell & infrastructure",
          items: [
            { text: "Overview", link: "/modules/" },
            { text: "entry", link: "/modules/entry" },
            { text: "core-ui", link: "/modules/core-ui" },
            { text: "topbar", link: "/modules/topbar" },
            { text: "statusbar", link: "/modules/statusbar" },
            { text: "panels", link: "/modules/panels" },
            { text: "settings", link: "/modules/settings" },
            { text: "game-settings", link: "/modules/game-settings" },
            { text: "asset-browser", link: "/modules/asset-browser" },
            { text: "scene-browser", link: "/modules/scene-browser" },
            { text: "sheet-fallback", link: "/modules/sheet-fallback" },
          ],
        },
        {
          text: "Gameplay",
          items: [
            { text: "stage", link: "/modules/stage" },
            { text: "scene-tools", link: "/modules/scene-tools" },
            { text: "actors", link: "/modules/actors" },
            { text: "factions", link: "/modules/factions" },
            { text: "conditions", link: "/modules/conditions" },
            { text: "chat", link: "/modules/chat" },
            { text: "chat-composer", link: "/modules/chat-composer" },
            { text: "chat-card", link: "/modules/chat-card" },
            { text: "combat-tracker", link: "/modules/combat-tracker" },
            { text: "notes", link: "/modules/notes" },
            { text: "tables", link: "/modules/tables" },
            { text: "sheet-actor", link: "/modules/sheet-actor" },
            { text: "sheet-item", link: "/modules/sheet-item" },
            { text: "sheet-note", link: "/modules/sheet-note" },
            { text: "sheet-table", link: "/modules/sheet-table" },
            { text: "audio", link: "/modules/audio" },
            { text: "sheet-playlist", link: "/modules/sheet-playlist" },
          ],
        },
      ],
    },
    search: { provider: "local" },
  },
});
