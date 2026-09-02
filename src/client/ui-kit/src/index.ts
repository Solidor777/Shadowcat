// Public surface of the shared Svelte UI runtime. The shell and every UI module
// import these seams from here — never from each other (the contract-only
// element boundary).
export { getAppContext, setAppContext, __APP_CONTEXT_KEY__ } from "./appContext";
export type { AppContext, TFunc } from "./appContext";
export { default as Surface } from "./Surface.svelte";
export { t, locale, i18n } from "./i18n.svelte";
export { notifications, activeNotifications } from "./notifications.svelte";
export { default as NotificationHost } from "./NotificationHost.svelte";
export { SceneInteractionBridge } from "./sceneInteraction";
export type { SceneInteraction } from "./sceneInteraction";
export { createMenuKeyboard } from "./MenuKeyboard";
export type { MenuKeyboard } from "./MenuKeyboard";
export { ActorSelection } from "./actorSelection.svelte";
export { TokenSelection } from "./tokenSelection.svelte";
export { SceneSelection } from "./sceneSelection.svelte";
export { SpeakAs } from "./speakAs.svelte";
export { SpeakAsToken } from "./speakAsToken.svelte";
export { sizeClass } from "./sizeClass.svelte";
export type { SizeClass } from "./sizeClass.svelte";
export { PanelsBridge } from "./panelsBridge.svelte";
export type { PanelsApi, PanelsChipsView } from "./panelsBridge.svelte";
export { setField } from "./sheetEdit";
export { default as SystemTreeEditor } from "./SystemTreeEditor.svelte";
export { SheetsController } from "./sheetsController.svelte";
export type { SheetsControllerDeps } from "./sheetsController.svelte";
export { default as MergeConflictModal } from "./MergeConflictModal.svelte";
export type { ConflictGroup } from "./mergeConflict";
export { TemplatesController } from "./templatesController.svelte";
export type { TemplatesControllerDeps, PendingSession } from "./templatesController.svelte";
export { AssetPickController } from "./assetPickController.svelte";
export type { PickAssetOptions, PickAssetMultiple, PendingPick } from "./assetPickController.svelte";
export { default as TemplateModalHost } from "./TemplateModalHost.svelte";
export type { ChatApi, TemplatesApi } from "./appContext";
export { default as TemplateControls } from "./TemplateControls.svelte";
export { default as SheetHost } from "./SheetHost.svelte";
export {
  THEME_TOKEN_NAMES,
  BUILTIN_THEMES,
  DEFAULT_THEME_ID,
  CONTRAST_PAIRINGS,
  THEME_ISOLATION_CLASS,
  THEME_ISOLATION_SHEET_ID,
  resolveTheme,
  sanitizeCustomTheme,
  sanitizeCustomThemes,
  colorThemeTokenNames,
  contrastWarnings,
  themeIsolationCss,
  wcagContrast,
} from "./theme";
export type { ThemeDefinition, ThemeTokenName, CustomTheme, ContrastPairing } from "./theme";
export { ThemeController, theme, activeTheme } from "./theme.svelte";
export type { PersistedTheme, ThemeListener } from "./theme.svelte";
