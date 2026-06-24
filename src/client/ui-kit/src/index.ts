// Public surface of the shared Svelte UI runtime. The shell and every UI module
// import these seams from here — never from each other (the contract-only
// element boundary; ARCHITECTURE.md §1).
export { getAppContext, setAppContext, __APP_CONTEXT_KEY__ } from "./appContext";
export type { AppContext, TFunc } from "./appContext";
export { default as Surface } from "./Surface.svelte";
export { t, locale, i18n } from "./i18n.svelte";
export { SceneInteractionBridge } from "./sceneInteraction";
export type { SceneInteraction } from "./sceneInteraction";
