export type { HealthStatus } from "./generated/HealthStatus";

// Document model
export type { Document } from "./generated/Document";
export type { Scope } from "./generated/Scope";
export type { Source } from "./generated/Source";
export type { PermissionSet } from "./generated/PermissionSet";
export type { CapabilityGrants } from "./generated/CapabilityGrants";
export type { DocRole } from "./generated/DocRole";
export type { Visibility } from "./generated/Visibility";
export type { WorldRole } from "./generated/WorldRole";

// Command / operation substrate
export type { Command } from "./generated/Command";
export type { Operation } from "./generated/Operation";
export type { FieldChange } from "./generated/FieldChange";

// WebSocket wire protocol
export type { ClientMsg } from "./generated/ClientMsg";
export type { ServerMsg } from "./generated/ServerMsg";
export type { RejectReason } from "./generated/RejectReason";
export type { ResyncSource } from "./generated/ResyncSource";
export type { WsErrorCode } from "./generated/WsErrorCode";
export type { Audience } from "./generated/Audience";

// Derived scene channels
export type { FootprintsPayload } from "./generated/FootprintsPayload";
export type { SceneFootprints } from "./generated/SceneFootprints";
export type { TokenFootprint } from "./generated/TokenFootprint";
export type { FootprintExtent } from "./generated/FootprintExtent";

// UI contribution contracts
export type { Cardinality } from "./generated/Cardinality";
export type { ContractProvide } from "./generated/ContractProvide";
export type { ContractDeclaration } from "./generated/ContractDeclaration";

// Structural schema registry
export type { SchemaType } from "./generated/SchemaType";
export type { Schema } from "./generated/Schema";
export type { SchemaDeclaration } from "./generated/SchemaDeclaration";

// HTTP API DTOs
export type { ServerConfig } from "./generated/ServerConfig";
export type { WorldEntry } from "./generated/WorldEntry";

// Module toolchain
export type { InstalledModuleInfo } from "./generated/InstalledModuleInfo";

// Assets
export type { Asset } from "./generated/Asset";
export type { AssetOp } from "./generated/AssetOp";

// Engine band (data/engine/*.rs) — server-validated per-doc-type bodies + shared shapes.
export type { TokenEngine } from "./generated/engine/TokenEngine";
export type { TokenOverrides } from "./generated/engine/TokenOverrides";
export type { TokenVisual } from "./generated/engine/TokenVisual";
export type { RenderVisual } from "./generated/engine/RenderVisual";
export type { AnimatedSource } from "./generated/engine/AnimatedSource";
export type { VisionAssignment } from "./generated/engine/VisionAssignment";
export type { Size } from "./generated/engine/Size";
export type { ActorEngine } from "./generated/engine/ActorEngine";
export type { SceneEngine } from "./generated/engine/SceneEngine";
export type { Grid } from "./generated/engine/Grid";
export type { GridDistance } from "./generated/engine/GridDistance";
export type { SceneDimensions } from "./generated/engine/SceneDimensions";
export type { SceneVisionOverrides } from "./generated/engine/SceneVisionOverrides";
export type { SceneLightingOverrides } from "./generated/engine/SceneLightingOverrides";
export type { EnvironmentLight } from "./generated/engine/EnvironmentLight";
export type { MovementModel } from "./generated/engine/MovementModel";
export type { MovementRestriction } from "./generated/engine/MovementRestriction";
export type { LightMode } from "./generated/engine/LightMode";
export type { DiagonalRule } from "./generated/engine/DiagonalRule";
export type { EasingMode } from "./generated/engine/EasingMode";
export type { WorldSettingsEngine } from "./generated/engine/WorldSettingsEngine";
export type { WorldSceneDefaults } from "./generated/engine/WorldSceneDefaults";
export type { Pathfinding } from "./generated/engine/Pathfinding";
export type { AnimationSettings } from "./generated/engine/AnimationSettings";
export type { LightEngine } from "./generated/engine/LightEngine";
export type { Falloff } from "./generated/engine/Falloff";
export type { GradationBand } from "./generated/engine/GradationBand";
export type { LightGradationEngine } from "./generated/engine/LightGradationEngine";
export type { VisionMode } from "./generated/engine/VisionMode";
export type { VisionModesEngine } from "./generated/engine/VisionModesEngine";
export type { WallEngine } from "./generated/engine/WallEngine";
export type { Seg } from "./generated/engine/Seg";
export type { RegionEngine } from "./generated/engine/RegionEngine";
export type { RegionShape } from "./generated/engine/RegionShape";
export type { DrawingEngine } from "./generated/engine/DrawingEngine";
export type { DrawingShape } from "./generated/engine/DrawingShape";
export type { Stroke } from "./generated/engine/Stroke";
export type { Fill } from "./generated/engine/Fill";
export type { TemplateEngine } from "./generated/engine/TemplateEngine";
export type { TemplateShape } from "./generated/engine/TemplateShape";
export type { Channel } from "./generated/engine/Channel";
export type { ChannelRegistryEngine } from "./generated/engine/ChannelRegistryEngine";
export type { Faction } from "./generated/engine/Faction";
export type { FactionStance } from "./generated/engine/FactionStance";
export type { FactionRegistryEngine } from "./generated/engine/FactionRegistryEngine";
export type { Condition } from "./generated/engine/Condition";
export type { ConditionRegistryEngine } from "./generated/engine/ConditionRegistryEngine";
export type { ChatSettingsEngine } from "./generated/engine/ChatSettingsEngine";
export type { DiceSettingsEngine } from "./generated/engine/DiceSettingsEngine";
export type { DiceModeSetting } from "./generated/engine/DiceModeSetting";
export type { DiceDirectionSetting } from "./generated/engine/DiceDirectionSetting";
export type { ChannelDiceOverride } from "./generated/engine/ChannelDiceOverride";
export type { CombatDefaults } from "./generated/engine/CombatDefaults";
export type { CombatEngine } from "./generated/engine/CombatEngine";
export type { CombatantEngine } from "./generated/engine/CombatantEngine";
export type { CombatantKind } from "./generated/engine/CombatantKind";
export type { CombatantResource } from "./generated/engine/CombatantResource";
export type { MovementRules } from "./generated/engine/MovementRules";
export type { Interpretation } from "./generated/engine/Interpretation";
export type { Enforcement } from "./generated/engine/Enforcement";
export type { TurnControl } from "./generated/engine/TurnControl";
export type { ResourceRegistryEngine } from "./generated/engine/ResourceRegistryEngine";
export type { Resource } from "./generated/engine/Resource";
export type { ResourceBinding } from "./generated/engine/ResourceBinding";
export type { Recovery } from "./generated/engine/Recovery";
export type { Formula } from "./generated/engine/Formula";
export type { EffectEngine } from "./generated/engine/EffectEngine";
export type { Duration } from "./generated/engine/Duration";
export type { DurationUnit } from "./generated/engine/DurationUnit";
export type { ExpiryPoint } from "./generated/engine/ExpiryPoint";
export type { ResolvedLifecycle } from "./generated/engine/ResolvedLifecycle";
export type { EffectLifecycle } from "./generated/engine/EffectLifecycle";
export type { EffectLifecycleDefaults } from "./generated/engine/EffectLifecycleDefaults";
export type { EffectSnapshot } from "./generated/engine/EffectSnapshot";
export type { CapturedCombatant } from "./generated/engine/CapturedCombatant";
export type { TurnRecord } from "./generated/engine/TurnRecord";
export type { CombatHistoryEngine } from "./generated/engine/CombatHistoryEngine";
export type { SystemDefaultsEngine } from "./generated/engine/SystemDefaultsEngine";
export type { SceneDefaultsOverlay } from "./generated/engine/SceneDefaultsOverlay";
export type { PathfindingOverlay } from "./generated/engine/PathfindingOverlay";
export type { AnimationOverlay } from "./generated/engine/AnimationOverlay";
