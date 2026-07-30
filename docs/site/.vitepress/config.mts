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
      "/modules/": [{ text: "First-party modules", items: [] }],
    },
    search: { provider: "local" },
  },
});
