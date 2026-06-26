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
  CapRequirement,
  HookDecl,
  ContractProvide,
  ContractDeclaration,
} from "./manifest";
export { ModuleRegistry } from "./modules";
export type { Module, ModuleContext, ModuleInfo } from "./modules";
export { loadModules } from "./loader";
export type { ImportFn, ModuleEntry } from "./loader";
export { resolveCaps, canWritePath } from "./capabilities";
export { DocumentStore, setPointer, applyOperation } from "./store";
export type { Listener, ReadableDocuments } from "./store";
export { ContributionRegistry } from "./contributions";
export type { Contribution, Cardinality } from "./contributions";
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
} from "./wire";
export { AssetResolver } from "./assets";
export type { AssetOp } from "./assets";
export { listAssets, uploadAsset, replaceAsset, deleteAsset } from "./asset-rest";
export { buildSceneDoc, buildTokenDoc, buildSceneEntityDoc, buildActorDoc, buildTokenFromActor, setNameHidden, buildFactionRegistryDoc, buildConditionRegistryDoc, buildWorldSettingsDoc, DEFAULT_WORLD_SETTINGS, resolveSceneSettings, DEFAULT_GRADATION, buildLightGradationDoc, resolveGradation, SEED_VISION_MODES, buildVisionModesDoc, resolveVisionModes, buildLightDoc } from "./scene-docs";
export type { SceneSystem, TokenSystem, ActorSystem, ActorVisual, TokenOverrides, Faction, FactionStance, FactionRegistrySystem, Condition, ConditionRegistrySystem, MovementRestriction, LightMode, DiagonalRule, EasingMode, EnvironmentLight, GridDistance, SceneVisionOverrides, SceneLightingOverrides, WorldSceneDefaults, WorldSettingsSystem, ResolvedSceneSettings, GradationBand, LightGradationSystem, VisionMode, VisionModesSystem, VisionAssignment, LightSystem } from "./scene-docs";
export { resolveTokenActor, actorDisplayName, resolveConditions, conditionTarget, resolveTokenBox, footprintRadius } from "./actor";
export type { EffectiveActor, ConditionTarget, TokenBox } from "./actor";
