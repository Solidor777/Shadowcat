import type { HealthStatus } from "@shadowcat/types";

/** Returns true when the server reports itself healthy with a live database. */
export function isHealthy(status: HealthStatus): boolean {
  return status.status === "ok" && status.db_connected;
}

export { silentLogger, consoleLogger } from "./logger";
export type { Logger } from "./logger";
export { HookBus, STOP } from "./hooks";
export type { HookKind, HookDefinition, OnOptions, Handler, CoreHooks } from "./hooks";
export { ServiceRegistry } from "./services";
export { MiddlewareChain } from "./middleware";
export type { PipelineName, Middleware } from "./middleware";
export { ManifestSchema, parseManifest, declarationOf } from "./manifest";
export type {
  ModuleManifest,
  ModuleEngines,
  CapRequirement,
  HookDecl,
  ContractProvide,
  ContractDeclaration,
} from "./manifest";
export { ModuleRegistry } from "./modules";
export type { Module, ModuleContext, ModuleInfo } from "./modules";
export { loadModules } from "./loader";
export type { ImportFn, ModuleEntry, ModuleLoadFailure, ModuleLoadResult } from "./loader";
export { resolveCaps, canWritePath } from "./capabilities";
export { DocumentStore, setPointer, getPointer, applyOperation } from "./store";
export type { Listener, ReadableDocuments } from "./store";
export { ContributionRegistry, PANEL_CONTRACT } from "./contributions";
export type { Contribution, Cardinality, PanelMeta, DefaultPlacement, ZoneId, SheetMeta } from "./contributions";
export { reconcileTopology } from "./topology";
export { I18n } from "./i18n";
export type { Messages, I18nParams } from "./i18n";
export { OptimisticClient } from "./optimistic";
export { WsClient } from "./ws-client";
export type {
  WsClientOptions,
  WsClientHandlers,
  WireWelcome,
  SearchPage,
  PathResult,
  MoveSample,
  MoveVisionSample,
  MoveStream,
  SubscriptionHandle,
  SceneFrame,
  SceneSubscription,
} from "./ws-client";
export { webSocketConnect } from "./transport";
export type { Transport, TransportHandlers, Connect } from "./transport";
export {
  parseServerMsg,
  DocumentSchema,
  CommandSchema,
  OperationSchema,
  ServerMsgSchema,
  CapabilityRequirementSchema,
  SearchHitSchema,
} from "./wire";
export type {
  ServerMsg,
  ClientMsg,
  WireDocument,
  WireCommand,
  WireOperation,
  WireFieldChange,
  WireScope,
  WireCapabilityRequirement,
  WireContractDeclaration,
  WireSearchHit,
  WireActorOwnerRef,
  WireAudience,
} from "./wire";
export { AssetResolver } from "./assets";
export type { AssetOp } from "./assets";
export { listAssets, uploadAsset, replaceAsset, deleteAsset } from "./asset-rest";
export { buildSceneDoc, buildTokenDoc, buildSceneEntityDoc, buildActorDoc, buildTokenFromActor, setNameHidden, buildFactionRegistryDoc, buildConditionRegistryDoc, buildWorldSettingsDoc, DEFAULT_WORLD_SETTINGS, resolveSceneSettings, resolveViewedScene, DEFAULT_GRADATION, buildLightGradationDoc, resolveGradation, SEED_VISION_MODES, buildVisionModesDoc, resolveVisionModes, buildLightDoc, buildRegionDoc, setRegionVisibility, DEFAULT_SCENE_BOUNDS, envelope, buildItemDoc, ITEM_DOC_TYPE } from "./scene-docs";
export type { SceneEngine, TokenEngine, ActorEngine, TokenOverrides, RenderVisual, AnimatedSource, FaceVisual, TokenVisual, Faction, FactionStance, FactionRegistryEngine, Condition, ConditionRegistryEngine, MovementRestriction, MovementModel, LightMode, DiagonalRule, EasingMode, EnvironmentLight, GridDistance, SceneVisionOverrides, SceneLightingOverrides, WorldSceneDefaults, WorldSettingsEngine, ResolvedSceneSettings, GradationBand, LightGradationEngine, VisionMode, VisionModesEngine, VisionAssignment, LightEngine, RegionShapeKind, RegionShape, RegionBehavior, RegionEngine, SceneDimensions, ItemSystem, DrawingEngine, DrawingShape, TemplateEngine, TemplateShape, Stroke, Fill, Grid, WallEngine, Seg } from "./scene-docs";
export { resolveTokenActor, actorDisplayName, resolveConditions, conditionTarget, resolveTokenBox, footprintRadius, resolveTokenVisual } from "./actor";
export type { EffectiveActor, ConditionTarget, TokenBox } from "./actor";
export { SHEET_CONTRACT_PREFIX, SHEET_FALLBACK_CONTRACT, sheetContract, resolveDocRef, pickSheet, isDiceNotation } from "./sheets";
export type { SheetRef, SheetTarget } from "./sheets";
export { MESSAGE_DOC_TYPE, CHANNEL_REGISTRY_DOC_TYPE, DICE_SETTINGS_DOC_TYPE, CHAT_SETTINGS_DOC_TYPE, MAX_MESSAGE_CHARS, MessageKindSchema, DieRecordSchema, RollOutcomeSchema, ChatSegmentSchema, ChatMessageEngineSchema, parseMessageEngine, isKnownSegment, buildChannelRegistryDoc, buildDiceSettingsDoc, buildChatSettingsDoc } from "./chat-docs";
export type { MessageKind, DieRecord, RollOutcome, ChatSegment, UnknownSegment, ChatMessageEngine, ChannelRegistryEngine, ChatSettingsEngine, DiceSettingsEngine } from "./chat-docs";
