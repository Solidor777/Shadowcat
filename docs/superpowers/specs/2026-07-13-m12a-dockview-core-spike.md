# M12a-0 — dockview-core source-verification spike (gate report)

**Pinned:** `dockview-core@7.0.2` (npm latest) = tag `v7.0.2`, commit `ecd409e2` (clean tag↔npm
match). **License:** MIT (verified in repo). Verified against the cloned TypeScript source at
`packages/dockview-core/src/` — not README/docs prose ([[verify-crate-claims-against-vendored-source]]).
Method: Sonnet subagent source read with file:line citations; the five gating citations were then
**independently re-read mainline** (dispatcher) from the same clone.

## Verdicts (spec §11)

| Q | Claim | Verdict |
|---|---|---|
| A | Zero runtime dependencies | PASS (`package.json` has no `dependencies` key) |
| B | Framework-agnostic DOM content API | PASS |
| C | Tabs + splits + drag-to-dock with drop previews | PASS |
| D | Floating groups | PASS |
| E | Pop-out: same-heap `window.open`, content re-parented | PASS |
| F | DOM re-parenting on dock⇄float/group moves | PASS |
| G | Full-layout serialization incl. floating + popout | PASS |
| H | Locked non-closable, non-drop-target center group | **PARTIAL** (see ruling) |
| I | PointerEvent/touch drag backend | PASS (dedicated coarse-pointer backend, `dndCapabilities.ts`) |
| J | Keyboard/ARIA baseline | PASS (own command-menu path still required per spec §9) |
| K | CSS-var theming, no hard-coded theme | PASS |
| L | Bundle weight | ~329 KB ESM min / ~73 KB gzip (import-everything ceiling; monolithic barrel — tree-shaking unverified, treat ceiling as the budget number) |
| M | MIT license | PASS |
| N | Maintenance cadence | PASS (active tags through v7.0.2) |

## Key evidence (mainline-verified)

- **Re-parenting (E/F)** — `src/overlay/overlayRenderContainer.ts` ~L150-163: caches
  `panel.view.content.element` and moves the SAME node via `appendChild`; ~L302-307 comment
  confirms hidden content stays in the DOM (scroll position preserved). Popout:
  `src/dockview/dockviewComponent.ts` ~L1512 `popoutContainer.appendChild(popoutGridview.element)`
  — existing subtree moved into the child window; `src/popoutWindow.ts` ~L112 plain
  `window.open(url, target, features)` (no `noopener` ⇒ same JS realm; null ⇒ popup-blocked path
  exists, matching spec §10's fallback requirement).
- **No-drop lock (H, verified half)** — `src/dockview/dockviewGroupPanelModel.ts` ~L1715:
  `if (this.locked === 'no-drop-target') return;` before any drop handling.
- **Veto hook (H, wrapper half)** — same file ~L1731-1745: `DockviewWillDropEvent` fired with
  `defaultPrevented` honored — the primitive the wrapper uses to enforce stage-well policy.

## Gate ruling (dispatcher, 2026-07-13)

**ADOPT.** All gating claims are source-verified except "non-closable center group," for which no
engine primitive exists. Ruling: the stage-well invariant is enforceable entirely with verified
primitives + wrapper policy, so H's gap converts into **mandatory M12a wrapper requirements**:

- **W1** — the stage mounts as a headerless/custom group: the wrapper exposes no close, drag,
  minimize, or pop-out affordance for it.
- **W2** — an `onWillDrop` veto rejects any drop that would relocate or displace the stage panel;
  the wrapper's own API (`ctx.panels.*`, command menu) refuses `close/move/float/minimize/popout`
  for the stage id.
- **W3** — fail-safe invariant guard: if the stage panel ever leaves the model, the wrapper
  restores it and logs. Dedicated tests attempt programmatic close/move/drop-displacement and
  assert the stage survives.

Residual (non-gating) notes for the M12a plan: tree-shaking is unproven — budget the full ~73 KB
gzip; keyboard model per spec §9 is ours to build; theming via our SCSS tokens over dockview's
CSS vars.
