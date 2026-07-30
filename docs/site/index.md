---
layout: home
hero:
  name: Shadowcat
  text: Self-hostable, fully moddable virtual tabletop
  tagline: One native executable. Server-authoritative. Built to be modded.
  actions:
    - theme: brand
      text: Host a server
      link: /guides/hosting
    - theme: alt
      text: Create a module
      link: /guides/creating-a-module
    - theme: alt
      text: Create a system
      link: /guides/creating-a-system
features:
  - title: Guides
    details: Step-by-step tutorials with complete, CI-built example code.
  - title: TypeScript API
    details: Generated reference for every workspace package — @shadowcat/core, ui-kit, formula, types, and all first-party modules.
    link: /api/ts/
  - title: Rust API
    details: Generated reference for the server crate, private items included.
    link: /api/rust/shadowcat/
---

## Reading these docs locally

The portal does not render from `file://` (VitePress emits absolute asset paths).
From a Shadowcat checkout run `pnpm docs:build` once, then `pnpm docs:serve` and
open the printed URL.
