// Shared test support for packages that consume the ui-kit seams. Exposed via
// the `@shadowcat/ui-kit/test` subpath (NOT the runtime barrel) so fixtures never
// reach a production bundle. Internal-only fixtures (SurfaceHarness, I18nProbe)
// stay unexported.
export { setAppContextForTest } from "./appContextTest";
export { fakeSceneHost } from "./fakeSceneHost";
export { default as Probe } from "./Probe.svelte";
