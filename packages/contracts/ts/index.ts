// Zod schemas for the API contracts. The Rust structs in `packages/contracts/src/` are the source
// of truth: `make contracts` regenerates the types in `ts/generated/`, and each schema below must
// satisfy its generated type, so drift fails `make type-check`.
//
// Identity DTOs come from lib-rustauth. `@rustauth/contracts` is pinned to the same commit as the
// `rustauth` crate (bump both together) and re-exported here, so `apps/web` imports only
// `@copilot/contracts`. Names defined in this file take precedence over the re-export: the shared
// envelopes, `errorResponseSchema`, and `REQUEST_ID_HEADER` match rustauth's (see
// `test/auth.test.ts`).
//
// Response schemas mirror what the server sends (every field, `null` for a missing value).
// Request schemas accept what the server accepts; the server stays the authority on rules and
// answers `400 VALIDATION_ERROR` with the failing `field`.

export * from "@rustauth/contracts";

import type { Approval } from "@contracts/generated/Approval";
import type { ApprovalMode } from "@contracts/generated/ApprovalMode";
import type { ApprovedTool } from "@contracts/generated/ApprovedTool";
import type { BuiltinTool } from "@contracts/generated/BuiltinTool";
import type { CallDecision } from "@contracts/generated/CallDecision";
import type { Capabilities } from "@contracts/generated/Capabilities";
import type { Capability } from "@contracts/generated/Capability";
import type { CapabilityLock } from "@contracts/generated/CapabilityLock";
import type { CapabilityOverrides } from "@contracts/generated/CapabilityOverrides";
import type { Capacity } from "@contracts/generated/Capacity";
import type { CapacityLevel } from "@contracts/generated/CapacityLevel";
import type { ChatListQuery } from "@contracts/generated/ChatListQuery";
import type { ChatResponse } from "@contracts/generated/ChatResponse";
import type { ChatStreamEvent } from "@contracts/generated/ChatStreamEvent";
import type { ChatSummaryResponse } from "@contracts/generated/ChatSummaryResponse";
import type { CreateChatRequest } from "@contracts/generated/CreateChatRequest";
import type { CreatedBy } from "@contracts/generated/CreatedBy";
import type { CreateProjectRequest } from "@contracts/generated/CreateProjectRequest";
import type { CreateProviderRequest } from "@contracts/generated/CreateProviderRequest";
import type { CreateScheduleRequest } from "@contracts/generated/CreateScheduleRequest";
import type { CreateToolServerRequest } from "@contracts/generated/CreateToolServerRequest";
import type { Credentials } from "@contracts/generated/Credentials";
import type { Decision } from "@contracts/generated/Decision";
import type { DecisionScope } from "@contracts/generated/DecisionScope";
import type { DecisionsRequest } from "@contracts/generated/DecisionsRequest";
import type { ErrorResponse } from "@contracts/generated/ErrorResponse";
import type { FileResponse } from "@contracts/generated/FileResponse";
import type { FinishReason } from "@contracts/generated/FinishReason";
import type { HealthResponse } from "@contracts/generated/HealthResponse";
import type { KeyState } from "@contracts/generated/KeyState";
import type { MessagePartDto } from "@contracts/generated/MessagePartDto";
import type { MessageResponse } from "@contracts/generated/MessageResponse";
import type { MessageStatus } from "@contracts/generated/MessageStatus";
import type { ModelResponse } from "@contracts/generated/ModelResponse";
import type { MoveFileRequest } from "@contracts/generated/MoveFileRequest";
import type { OnAsk } from "@contracts/generated/OnAsk";
import type { PartKind } from "@contracts/generated/PartKind";
import type { Preset } from "@contracts/generated/Preset";
import type { ProjectDetailsResponse } from "@contracts/generated/ProjectDetailsResponse";
import type { ProjectListQuery } from "@contracts/generated/ProjectListQuery";
import type { ProjectResponse } from "@contracts/generated/ProjectResponse";
import type { ProviderApi } from "@contracts/generated/ProviderApi";
import type { ProviderDetailsResponse } from "@contracts/generated/ProviderDetailsResponse";
import type { ProviderKind } from "@contracts/generated/ProviderKind";
import type { ProviderResponse } from "@contracts/generated/ProviderResponse";
import type { ProviderUsage } from "@contracts/generated/ProviderUsage";
import type { ReadyResponse } from "@contracts/generated/ReadyResponse";
import type { ReasoningLevel } from "@contracts/generated/ReasoningLevel";
import type { RefreshResponse } from "@contracts/generated/RefreshResponse";
import type { Refusal } from "@contracts/generated/Refusal";
import type { ResultContent } from "@contracts/generated/ResultContent";
import type { Role } from "@contracts/generated/Role";
import type { RunStatus } from "@contracts/generated/RunStatus";
import type { ScheduleListQuery } from "@contracts/generated/ScheduleListQuery";
import type { SchedulePreviewRequest } from "@contracts/generated/SchedulePreviewRequest";
import type { SchedulePreviewResponse } from "@contracts/generated/SchedulePreviewResponse";
import type { ScheduleResponse } from "@contracts/generated/ScheduleResponse";
import type { ScheduleRunResponse } from "@contracts/generated/ScheduleRunResponse";
import type { SearchHitResponse } from "@contracts/generated/SearchHitResponse";
import type { SearchQuery } from "@contracts/generated/SearchQuery";
import type { SendMessageRequest } from "@contracts/generated/SendMessageRequest";
import type { ServerTool } from "@contracts/generated/ServerTool";
import type { SettingsResponse } from "@contracts/generated/SettingsResponse";
import type { StreamQuery } from "@contracts/generated/StreamQuery";
import type { ToolChanges } from "@contracts/generated/ToolChanges";
import type { ToolMode } from "@contracts/generated/ToolMode";
import type { ToolOverride } from "@contracts/generated/ToolOverride";
import type { ToolServerResponse } from "@contracts/generated/ToolServerResponse";
import type { ToolStatus } from "@contracts/generated/ToolStatus";
import type { ToolTestResponse } from "@contracts/generated/ToolTestResponse";
import type { UpdateChatRequest } from "@contracts/generated/UpdateChatRequest";
import type { UpdateFileRequest } from "@contracts/generated/UpdateFileRequest";
import type { UpdateModelRequest } from "@contracts/generated/UpdateModelRequest";
import type { UpdateProjectRequest } from "@contracts/generated/UpdateProjectRequest";
import type { UpdateProviderRequest } from "@contracts/generated/UpdateProviderRequest";
import type { UpdateScheduleRequest } from "@contracts/generated/UpdateScheduleRequest";
import type { UpdateSettingsRequest } from "@contracts/generated/UpdateSettingsRequest";
import type { UpdateToolServerRequest } from "@contracts/generated/UpdateToolServerRequest";
import type { Usage } from "@contracts/generated/Usage";
import type { UserEvent } from "@contracts/generated/UserEvent";
import { z } from "zod";

export type { Approval } from "@contracts/generated/Approval";
export type { ApprovalMode } from "@contracts/generated/ApprovalMode";
export type { ApprovedTool } from "@contracts/generated/ApprovedTool";
export type { BuiltinTool } from "@contracts/generated/BuiltinTool";
export type { CallDecision } from "@contracts/generated/CallDecision";
export type { Capabilities } from "@contracts/generated/Capabilities";
export type { Capability } from "@contracts/generated/Capability";
export type { CapabilityLock } from "@contracts/generated/CapabilityLock";
export type { CapabilityOverrides } from "@contracts/generated/CapabilityOverrides";
export type { Capacity } from "@contracts/generated/Capacity";
export type { CapacityLevel } from "@contracts/generated/CapacityLevel";
export type { ChatListQuery } from "@contracts/generated/ChatListQuery";
export type { ChatResponse } from "@contracts/generated/ChatResponse";
export type { ChatStreamEvent } from "@contracts/generated/ChatStreamEvent";
export type { ChatSummaryResponse } from "@contracts/generated/ChatSummaryResponse";
export type { CreateChatRequest } from "@contracts/generated/CreateChatRequest";
export type { CreatedBy } from "@contracts/generated/CreatedBy";
export type { CreateProjectRequest } from "@contracts/generated/CreateProjectRequest";
export type { CreateProviderRequest } from "@contracts/generated/CreateProviderRequest";
export type { CreateScheduleRequest } from "@contracts/generated/CreateScheduleRequest";
export type { CreateToolServerRequest } from "@contracts/generated/CreateToolServerRequest";
export type { Credentials } from "@contracts/generated/Credentials";
export type { Decision } from "@contracts/generated/Decision";
export type { DecisionScope } from "@contracts/generated/DecisionScope";
export type { DecisionsRequest } from "@contracts/generated/DecisionsRequest";
export type { DetailsResponse } from "@contracts/generated/DetailsResponse";
export type { ErrorResponse } from "@contracts/generated/ErrorResponse";
export type { FileResponse } from "@contracts/generated/FileResponse";
export type { FinishReason } from "@contracts/generated/FinishReason";
export type { HealthResponse } from "@contracts/generated/HealthResponse";
export type { KeyState } from "@contracts/generated/KeyState";
export type { ListResponse } from "@contracts/generated/ListResponse";
export type { MessagePartDto } from "@contracts/generated/MessagePartDto";
export type { MessageResponse } from "@contracts/generated/MessageResponse";
export type { MessageStatus } from "@contracts/generated/MessageStatus";
export type { ModelResponse } from "@contracts/generated/ModelResponse";
export type { MoveFileRequest } from "@contracts/generated/MoveFileRequest";
export type { OnAsk } from "@contracts/generated/OnAsk";
export type { PageResponse } from "@contracts/generated/PageResponse";
export type { PartKind } from "@contracts/generated/PartKind";
export type { Preset } from "@contracts/generated/Preset";
export type { ProjectDetailsResponse } from "@contracts/generated/ProjectDetailsResponse";
export type { ProjectListQuery } from "@contracts/generated/ProjectListQuery";
export type { ProjectResponse } from "@contracts/generated/ProjectResponse";
export type { ProviderApi } from "@contracts/generated/ProviderApi";
export type { ProviderDetailsResponse } from "@contracts/generated/ProviderDetailsResponse";
export type { ProviderKind } from "@contracts/generated/ProviderKind";
export type { ProviderResponse } from "@contracts/generated/ProviderResponse";
export type { ProviderUsage } from "@contracts/generated/ProviderUsage";
export type { ReadyResponse } from "@contracts/generated/ReadyResponse";
export type { ReasoningLevel } from "@contracts/generated/ReasoningLevel";
export type { RefreshResponse } from "@contracts/generated/RefreshResponse";
export type { Refusal } from "@contracts/generated/Refusal";
export type { ResultContent } from "@contracts/generated/ResultContent";
export type { Role } from "@contracts/generated/Role";
export type { RunStatus } from "@contracts/generated/RunStatus";
export type { ScheduleListQuery } from "@contracts/generated/ScheduleListQuery";
export type { SchedulePreviewRequest } from "@contracts/generated/SchedulePreviewRequest";
export type { SchedulePreviewResponse } from "@contracts/generated/SchedulePreviewResponse";
export type { ScheduleResponse } from "@contracts/generated/ScheduleResponse";
export type { ScheduleRunResponse } from "@contracts/generated/ScheduleRunResponse";
export type { SearchHitResponse } from "@contracts/generated/SearchHitResponse";
export type { SearchQuery } from "@contracts/generated/SearchQuery";
export type { SendMessageRequest } from "@contracts/generated/SendMessageRequest";
export type { ServerTool } from "@contracts/generated/ServerTool";
export type { SettingsResponse } from "@contracts/generated/SettingsResponse";
export type { StreamQuery } from "@contracts/generated/StreamQuery";
export type { ToolChanges } from "@contracts/generated/ToolChanges";
export type { ToolMode } from "@contracts/generated/ToolMode";
export type { ToolOverride } from "@contracts/generated/ToolOverride";
export type { ToolServerResponse } from "@contracts/generated/ToolServerResponse";
export type { ToolStatus } from "@contracts/generated/ToolStatus";
export type { ToolTestResponse } from "@contracts/generated/ToolTestResponse";
export type { UpdateChatRequest } from "@contracts/generated/UpdateChatRequest";
export type { UpdateFileRequest } from "@contracts/generated/UpdateFileRequest";
export type { UpdateModelRequest } from "@contracts/generated/UpdateModelRequest";
export type { UpdateProjectRequest } from "@contracts/generated/UpdateProjectRequest";
export type { UpdateProviderRequest } from "@contracts/generated/UpdateProviderRequest";
export type { UpdateScheduleRequest } from "@contracts/generated/UpdateScheduleRequest";
export type { UpdateSettingsRequest } from "@contracts/generated/UpdateSettingsRequest";
export type { UpdateToolServerRequest } from "@contracts/generated/UpdateToolServerRequest";
export type { Usage } from "@contracts/generated/Usage";
export type { UserEvent } from "@contracts/generated/UserEvent";

export const REQUEST_ID_HEADER = "X-Request-Id";

/** The multipart field of `POST /projects/{id}/files` and `POST /chats/{id}/files`. */
export const UPLOAD_FIELD = "file";

// Envelopes and health

export const healthResponseSchema = z.object({
  status: z.string(),
}) satisfies z.ZodType<HealthResponse>;

export const readyResponseSchema = z.object({
  status: z.string(),
  database: z.string(),
}) satisfies z.ZodType<ReadyResponse>;

export const errorResponseSchema = z.object({
  error: z.string(),
  code: z.string().optional(),
  field: z.string().optional(),
  details: z.record(z.string(), z.json()).optional(),
  requestId: z.string().optional(),
}) satisfies z.ZodType<ErrorResponse>;

/** `{ items, total }` list envelope around `item`. */
export function listResponseSchema<T extends z.ZodType>(item: T) {
  return z.object({
    items: z.array(item),
    total: z.number().int().nonnegative(),
  });
}

/** `{ data }` details envelope around `data`. Pass `schema.nullable()` when absence is valid. */
export function detailsResponseSchema<T extends z.ZodType>(data: T) {
  return z.object({ data });
}

/**
 * `{ items, next }` cursor page around `item`. Pass `next` back as `cursor` for the following
 * page; it is `null` on the last page.
 */
export function pageResponseSchema<T extends z.ZodType>(item: T) {
  return z.object({
    items: z.array(item),
    next: z.string().nullable(),
  });
}

// Shared values

const id = z.uuid();
const timestamp = z.iso.datetime({ offset: true });
const count = z.number().int().nonnegative();

export const reasoningLevelSchema = z.enum([
  "off",
  "low",
  "medium",
  "high",
]) satisfies z.ZodType<ReasoningLevel>;

export const builtinToolSchema = z.enum(["schedules"]) satisfies z.ZodType<BuiltinTool>;

export const keyStateSchema = z.enum([
  "missing",
  "set",
  "unreadable",
]) satisfies z.ZodType<KeyState>;

// Settings

export const settingsResponseSchema = z.object({
  instructions: z.string(),
}) satisfies z.ZodType<SettingsResponse>;

export const updateSettingsRequestSchema = z.object({
  instructions: z.string().optional(),
}) satisfies z.ZodType<UpdateSettingsRequest>;

// Providers

export const providerKindSchema = z.enum(["openai", "anthropic"]) satisfies z.ZodType<ProviderKind>;

export const providerApiSchema = z.enum([
  "chat_completions",
  "responses",
]) satisfies z.ZodType<ProviderApi>;

export const presetSchema = z.enum([
  "openai",
  "anthropic",
  "gemini",
  "openrouter",
  "ollama",
  "custom",
]) satisfies z.ZodType<Preset>;

export const providerResponseSchema = z.object({
  id,
  kind: providerKindSchema,
  preset: presetSchema,
  api: providerApiSchema.nullable(),
  name: z.string(),
  baseUrl: z.string(),
  enabled: z.boolean(),
  key: keyStateSchema,
  createdAt: timestamp,
  updatedAt: timestamp,
}) satisfies z.ZodType<ProviderResponse>;

export const providerUsageSchema = z.object({
  chats: count,
  projects: count,
  schedules: count,
}) satisfies z.ZodType<ProviderUsage>;

export const providerDetailsResponseSchema = providerResponseSchema.extend({
  usage: providerUsageSchema,
}) satisfies z.ZodType<ProviderDetailsResponse>;

export const createProviderRequestSchema = z.object({
  kind: providerKindSchema,
  preset: presetSchema,
  api: providerApiSchema.optional(),
  name: z.string(),
  baseUrl: z.string().optional(),
  apiKey: z.string().optional(),
}) satisfies z.ZodType<CreateProviderRequest>;

export const updateProviderRequestSchema = z.object({
  name: z.string().optional(),
  api: providerApiSchema.optional(),
  baseUrl: z.string().optional(),
  apiKey: z.string().optional(),
  enabled: z.boolean().optional(),
}) satisfies z.ZodType<UpdateProviderRequest>;

export const refreshResponseSchema = z.object({
  added: count,
  updated: count,
  unavailable: count,
  enabled: count,
}) satisfies z.ZodType<RefreshResponse>;

// Models

export const capabilitySchema = z.enum([
  "vision",
  "pdf",
  "tools",
  "audioIn",
  "audioOut",
  "reasoning",
]) satisfies z.ZodType<Capability>;

export const capabilitiesSchema = z.object({
  vision: z.boolean(),
  pdf: z.boolean(),
  tools: z.boolean(),
  audioIn: z.boolean(),
  audioOut: z.boolean(),
  reasoning: z.boolean(),
  reasoningLevels: z.array(reasoningLevelSchema),
  defaultReasoning: reasoningLevelSchema.nullable(),
}) satisfies z.ZodType<Capabilities>;

export const capabilityOverridesSchema = z.object({
  vision: z.boolean().nullable(),
  pdf: z.boolean().nullable(),
  tools: z.boolean().nullable(),
  audioIn: z.boolean().nullable(),
  audioOut: z.boolean().nullable(),
  reasoning: z.boolean().nullable(),
}) satisfies z.ZodType<CapabilityOverrides>;

export const capabilityLockSchema = z.object({
  capability: capabilitySchema,
  reason: z.string(),
}) satisfies z.ZodType<CapabilityLock>;

export const modelResponseSchema = z.object({
  id,
  providerId: id,
  providerName: z.string(),
  kind: providerKindSchema,
  preset: presetSchema,
  remoteId: z.string(),
  displayName: z.string(),
  enabled: z.boolean(),
  available: z.boolean(),
  contextWindow: count.nullable(),
  maxOutputTokens: count.nullable(),
  capabilities: capabilitiesSchema,
  detected: capabilitiesSchema,
  overrides: capabilityOverridesSchema,
  locks: z.array(capabilityLockSchema),
  reasoningOffWithTools: z.boolean(),
  createdAt: timestamp,
  updatedAt: timestamp,
}) satisfies z.ZodType<ModelResponse>;

export const updateModelRequestSchema = z.object({
  enabled: z.boolean().optional(),
  displayName: z.string().optional(),
  capabilityOverrides: capabilityOverridesSchema.optional(),
  contextWindow: count.nullable().optional(),
  maxOutputTokens: count.nullable().optional(),
}) satisfies z.ZodType<UpdateModelRequest>;

// Files

export const fileResponseSchema = z.object({
  id,
  projectId: id.nullable(),
  chatId: id.nullable(),
  name: z.string(),
  mediaType: z.string(),
  sizeBytes: count,
  tokenEstimate: count,
  pageCount: count.nullable(),
  included: z.boolean(),
  createdAt: timestamp,
}) satisfies z.ZodType<FileResponse>;

export const updateFileRequestSchema = z.object({
  name: z.string().optional(),
  included: z.boolean().optional(),
}) satisfies z.ZodType<UpdateFileRequest>;

export const moveFileRequestSchema = z.object({
  projectId: id,
}) satisfies z.ZodType<MoveFileRequest>;

export const capacityLevelSchema = z.enum([
  "unknown",
  "fits",
  "warn",
  "over",
]) satisfies z.ZodType<CapacityLevel>;

export const capacitySchema = z.object({
  used: count,
  window: count.nullable(),
  level: capacityLevelSchema,
}) satisfies z.ZodType<Capacity>;

// Projects

export const projectResponseSchema = z.object({
  id,
  name: z.string(),
  icon: z.string(),
  instructions: z.string(),
  defaultModelId: id.nullable(),
  reasoning: reasoningLevelSchema.nullable(),
  builtinTools: z.array(builtinToolSchema),
  toolServerIds: z.array(id),
  pinned: z.boolean(),
  createdAt: timestamp,
  updatedAt: timestamp,
}) satisfies z.ZodType<ProjectResponse>;

export const projectDetailsResponseSchema = projectResponseSchema.extend({
  capacity: capacitySchema,
}) satisfies z.ZodType<ProjectDetailsResponse>;

export const projectListQuerySchema = z.object({
  pinned: z.boolean().optional(),
  limit: count.optional(),
}) satisfies z.ZodType<ProjectListQuery>;

export const createProjectRequestSchema = z.object({
  name: z.string(),
  icon: z.string().optional(),
  instructions: z.string().optional(),
  defaultModelId: id.optional(),
  reasoning: reasoningLevelSchema.optional(),
  builtinTools: z.array(builtinToolSchema).optional(),
  toolServerIds: z.array(id).optional(),
  pinned: z.boolean().optional(),
}) satisfies z.ZodType<CreateProjectRequest>;

export const updateProjectRequestSchema = z.object({
  name: z.string().optional(),
  icon: z.string().optional(),
  instructions: z.string().optional(),
  defaultModelId: id.nullable().optional(),
  reasoning: reasoningLevelSchema.nullable().optional(),
  builtinTools: z.array(builtinToolSchema).optional(),
  toolServerIds: z.array(id).optional(),
  pinned: z.boolean().optional(),
}) satisfies z.ZodType<UpdateProjectRequest>;

// Tool servers

export const approvalModeSchema = z.enum(["ask", "auto"]) satisfies z.ZodType<ApprovalMode>;

export const toolModeSchema = z.enum(["off", "ask", "auto"]) satisfies z.ZodType<ToolMode>;

export const serverToolSchema = z.object({
  name: z.string(),
  title: z.string().nullable(),
  description: z.string(),
  inputSchema: z.json(),
  readOnly: z.boolean(),
}) satisfies z.ZodType<ServerTool>;

export const toolServerResponseSchema = z.object({
  id,
  name: z.string(),
  slug: z.string(),
  url: z.string(),
  approval: approvalModeSchema,
  toolModes: z.record(z.string(), toolModeSchema),
  enabled: z.boolean(),
  credentials: keyStateSchema,
  protocolVersion: z.string().nullable(),
  tools: z.array(serverToolSchema).nullable(),
  toolsFetchedAt: timestamp.nullable(),
  reviewRequired: z.boolean(),
  createdAt: timestamp,
  updatedAt: timestamp,
}) satisfies z.ZodType<ToolServerResponse>;

export const createToolServerRequestSchema = z.object({
  name: z.string(),
  slug: z.string().optional(),
  url: z.string(),
  bearer: z.string().optional(),
  headers: z.record(z.string(), z.string()).default({}),
  approval: approvalModeSchema.optional(),
}) satisfies z.ZodType<CreateToolServerRequest>;

export const credentialsSchema = z.object({
  bearer: z.string().optional(),
  headers: z.record(z.string(), z.string()).default({}),
}) satisfies z.ZodType<Credentials>;

export const updateToolServerRequestSchema = z.object({
  name: z.string().optional(),
  url: z.string().optional(),
  credentials: credentialsSchema.nullable().optional(),
  approval: approvalModeSchema.optional(),
  toolModes: z.record(z.string(), toolModeSchema).optional(),
  enabled: z.boolean().optional(),
}) satisfies z.ZodType<UpdateToolServerRequest>;

export const toolChangesSchema = z.object({
  added: z.array(z.string()),
  removed: z.array(z.string()),
  changed: z.array(z.string()),
}) satisfies z.ZodType<ToolChanges>;

export const toolTestResponseSchema = z.object({
  server: toolServerResponseSchema,
  changes: toolChangesSchema,
}) satisfies z.ZodType<ToolTestResponse>;

// Schedules

export const onAskSchema = z.enum(["deny", "pause"]) satisfies z.ZodType<OnAsk>;

export const createdBySchema = z.enum(["user", "assistant"]) satisfies z.ZodType<CreatedBy>;

export const runStatusSchema = z.enum([
  "ok",
  "error",
  "awaiting_approval",
]) satisfies z.ZodType<RunStatus>;

export const scheduleResponseSchema = z.object({
  id,
  projectId: id.nullable(),
  modelId: id.nullable(),
  sourceChatId: id.nullable(),
  name: z.string(),
  cron: z.string(),
  timezone: z.string(),
  description: z.string(),
  prompt: z.string(),
  instructions: z.string(),
  reasoning: reasoningLevelSchema.nullable(),
  onAsk: onAskSchema,
  createdBy: createdBySchema,
  enabled: z.boolean(),
  nextRunAt: timestamp.nullable(),
  lastRunAt: timestamp.nullable(),
  lastStatus: runStatusSchema.nullable(),
  lastError: z.string().nullable(),
  consecutiveFailures: count,
  disabledReason: z.string().nullable(),
  createdAt: timestamp,
  updatedAt: timestamp,
}) satisfies z.ZodType<ScheduleResponse>;

export const scheduleListQuerySchema = z.object({
  projectId: id.optional(),
}) satisfies z.ZodType<ScheduleListQuery>;

export const createScheduleRequestSchema = z.object({
  name: z.string(),
  cron: z.string(),
  timezone: z.string(),
  prompt: z.string(),
  projectId: id.optional(),
  modelId: id.optional(),
  instructions: z.string().optional(),
  reasoning: reasoningLevelSchema.optional(),
  onAsk: onAskSchema.optional(),
}) satisfies z.ZodType<CreateScheduleRequest>;

export const updateScheduleRequestSchema = z.object({
  name: z.string().optional(),
  cron: z.string().optional(),
  timezone: z.string().optional(),
  prompt: z.string().optional(),
  instructions: z.string().optional(),
  modelId: id.optional(),
  reasoning: reasoningLevelSchema.nullable().optional(),
  onAsk: onAskSchema.optional(),
  enabled: z.boolean().optional(),
}) satisfies z.ZodType<UpdateScheduleRequest>;

export const schedulePreviewRequestSchema = z.object({
  cron: z.string(),
  timezone: z.string(),
}) satisfies z.ZodType<SchedulePreviewRequest>;

export const schedulePreviewResponseSchema = z.object({
  description: z.string(),
  timezone: z.string(),
  nextRuns: z.array(timestamp),
}) satisfies z.ZodType<SchedulePreviewResponse>;

export const scheduleRunResponseSchema = z.object({
  chatId: id,
}) satisfies z.ZodType<ScheduleRunResponse>;

// Messages

export const roleSchema = z.enum(["user", "assistant", "tool"]) satisfies z.ZodType<Role>;

export const messageStatusSchema = z.enum([
  "streaming",
  "complete",
  "awaiting_approval",
  "cancelled",
  "error",
  "refused",
]) satisfies z.ZodType<MessageStatus>;

export const finishReasonSchema = z.enum([
  "stop",
  "tool_calls",
  "length",
  "refusal",
  "error",
  "other",
  "tool_limit",
]) satisfies z.ZodType<FinishReason>;

export const toolStatusSchema = z.enum([
  "ok",
  "error",
  "denied",
  "cancelled",
  "timeout",
]) satisfies z.ZodType<ToolStatus>;

export const approvalSchema = z.enum([
  "auto",
  "pending",
  "approved",
  "denied",
]) satisfies z.ZodType<Approval>;

export const usageSchema = z.object({
  inputTokens: count.nullable(),
  outputTokens: count.nullable(),
  reasoningTokens: count.nullable(),
  cacheReadTokens: count.nullable(),
  cacheWriteTokens: count.nullable(),
}) satisfies z.ZodType<Usage>;

export const resultContentSchema = z.discriminatedUnion("type", [
  z.object({ type: z.literal("text"), text: z.string() }),
  z.object({ type: z.literal("image"), fileId: id }),
]) satisfies z.ZodType<ResultContent>;

export const messagePartSchema = z.discriminatedUnion("type", [
  z.object({ type: z.literal("text"), text: z.string() }),
  z.object({ type: z.literal("file"), fileId: id, name: z.string(), mediaType: z.string() }),
  z.object({
    type: z.literal("audio"),
    fileId: id,
    format: z.string(),
    transcript: z.string().nullable(),
    durationMs: count.nullable(),
  }),
  z.object({ type: z.literal("reasoning"), text: z.string() }),
  z.object({
    type: z.literal("toolCall"),
    callId: id,
    serverId: id.nullable(),
    toolName: z.string(),
    arguments: z.json(),
    complete: z.boolean(),
    approval: approvalSchema,
    denyReason: z.string().nullable(),
  }),
  z.object({
    type: z.literal("toolResult"),
    callId: id,
    status: toolStatusSchema,
    isError: z.boolean(),
    content: z.array(resultContentSchema),
    truncated: z.boolean(),
  }),
]) satisfies z.ZodType<MessagePartDto>;

export const messageResponseSchema = z.object({
  id,
  chatId: id,
  role: roleSchema,
  status: messageStatusSchema,
  finishReason: finishReasonSchema.nullable(),
  modelId: id.nullable(),
  parts: z.array(messagePartSchema),
  usage: usageSchema.nullable(),
  error: z.string().nullable(),
  createdAt: timestamp,
  updatedAt: timestamp,
}) satisfies z.ZodType<MessageResponse>;

// Chats and search

export const toolOverrideSchema = z.object({
  serverId: id,
  enabled: z.boolean(),
}) satisfies z.ZodType<ToolOverride>;

export const approvedToolSchema = z.object({
  serverId: id,
  name: z.string(),
}) satisfies z.ZodType<ApprovedTool>;

export const chatResponseSchema = z.object({
  id,
  projectId: id.nullable(),
  modelId: id.nullable(),
  scheduleId: id.nullable(),
  title: z.string(),
  instructions: z.string(),
  reasoning: reasoningLevelSchema.nullable(),
  builtinTools: z.array(builtinToolSchema).nullable(),
  toolOverrides: z.array(toolOverrideSchema),
  approvedTools: z.array(approvedToolSchema),
  pinned: z.boolean(),
  voiceReplies: z.boolean(),
  lastReadAt: timestamp.nullable(),
  createdAt: timestamp,
  updatedAt: timestamp,
}) satisfies z.ZodType<ChatResponse>;

export const chatSummaryResponseSchema = z.object({
  id,
  projectId: id.nullable(),
  modelId: id.nullable(),
  scheduleId: id.nullable(),
  title: z.string(),
  pinned: z.boolean(),
  unread: z.boolean(),
  awaitingApproval: z.boolean(),
  createdAt: timestamp,
  updatedAt: timestamp,
}) satisfies z.ZodType<ChatSummaryResponse>;

export const chatListQuerySchema = z.object({
  projectId: id.optional(),
  scheduleId: id.optional(),
  pinned: z.boolean().optional(),
  scheduled: z.boolean().optional(),
  cursor: z.string().optional(),
  limit: count.optional(),
}) satisfies z.ZodType<ChatListQuery>;

export const createChatRequestSchema = z.object({
  projectId: id.optional(),
  modelId: id.optional(),
  instructions: z.string().optional(),
  reasoning: reasoningLevelSchema.optional(),
  builtinTools: z.array(builtinToolSchema).optional(),
  toolOverrides: z.array(toolOverrideSchema).optional(),
  voiceReplies: z.boolean().optional(),
}) satisfies z.ZodType<CreateChatRequest>;

export const updateChatRequestSchema = z.object({
  title: z.string().optional(),
  projectId: id.nullable().optional(),
  modelId: id.nullable().optional(),
  instructions: z.string().optional(),
  reasoning: reasoningLevelSchema.nullable().optional(),
  builtinTools: z.array(builtinToolSchema).nullable().optional(),
  toolOverrides: z.array(toolOverrideSchema).optional(),
  pinned: z.boolean().optional(),
  voiceReplies: z.boolean().optional(),
}) satisfies z.ZodType<UpdateChatRequest>;

export const sendMessageRequestSchema = z.object({
  text: z.string(),
  fileIds: z.array(id).default([]),
  editFrom: id.optional(),
}) satisfies z.ZodType<SendMessageRequest>;

export const decisionSchema = z.enum(["approve", "deny"]) satisfies z.ZodType<Decision>;

export const decisionScopeSchema = z.enum(["once", "chat"]) satisfies z.ZodType<DecisionScope>;

export const callDecisionSchema = z.object({
  callId: id,
  decision: decisionSchema,
  scope: decisionScopeSchema.optional(),
  reason: z.string().optional(),
}) satisfies z.ZodType<CallDecision>;

export const decisionsRequestSchema = z.object({
  decisions: z.array(callDecisionSchema),
}) satisfies z.ZodType<DecisionsRequest>;

export const streamQuerySchema = z.object({
  after: count.optional(),
}) satisfies z.ZodType<StreamQuery>;

export const searchQuerySchema = z.object({
  q: z.string(),
  limit: count.optional(),
}) satisfies z.ZodType<SearchQuery>;

export const searchHitResponseSchema = z.object({
  chatId: id,
  chatTitle: z.string(),
  messageId: id.nullable(),
  snippet: z.string(),
}) satisfies z.ZodType<SearchHitResponse>;

// Stream events: the data of each server-sent event.

export const partKindSchema = z.enum([
  "text",
  "reasoning",
  "toolCall",
  "audio",
]) satisfies z.ZodType<PartKind>;

export const refusalSchema = z.object({
  category: z.string().nullable(),
  explanation: z.string(),
}) satisfies z.ZodType<Refusal>;

const seq = count;

/** Every event of a chat run (`POST /chats/{id}/messages` and the other streaming routes). */
export const chatStreamEventSchema = z.discriminatedUnion("type", [
  z.object({
    type: z.literal("message.started"),
    seq,
    messageId: id,
    role: roleSchema,
    windowStartId: id.nullable(),
    unavailable: z.array(z.string()).optional(),
  }),
  z.object({
    type: z.literal("part.started"),
    seq,
    messageId: id,
    index: count,
    kind: partKindSchema,
  }),
  z.object({
    type: z.literal("text.delta"),
    seq,
    messageId: id,
    index: count,
    text: z.string(),
  }),
  z.object({
    type: z.literal("reasoning.delta"),
    seq,
    messageId: id,
    index: count,
    text: z.string(),
  }),
  z.object({
    type: z.literal("audio.delta"),
    seq,
    messageId: id,
    data: z.string(),
    format: z.string(),
  }),
  z.object({
    type: z.literal("tool.requested"),
    seq,
    messageId: id,
    callId: id,
    serverId: id.nullable(),
    toolName: z.string(),
    title: z.string(),
    arguments: z.json(),
    needsApproval: z.boolean(),
  }),
  z.object({
    type: z.literal("tool.finished"),
    seq,
    messageId: id,
    callId: id,
    status: toolStatusSchema,
    output: z.array(resultContentSchema),
    truncated: z.boolean(),
  }),
  z.object({
    type: z.literal("message.finished"),
    seq,
    messageId: id,
    status: messageStatusSchema,
    finishReason: finishReasonSchema.nullable(),
    refusal: refusalSchema.nullable(),
    usage: usageSchema.nullable(),
  }),
  z.object({ type: z.literal("error"), seq, message: z.string(), code: z.string() }),
  z.object({ type: z.literal("done"), seq }),
]) satisfies z.ZodType<ChatStreamEvent>;

/** Every event of the caller's `GET /events` stream. */
export const userEventSchema = z.discriminatedUnion("type", [
  z.object({ type: z.literal("chat.updated"), chatId: id }),
  z.object({ type: z.literal("run.started"), chatId: id }),
  z.object({ type: z.literal("run.finished"), chatId: id, status: messageStatusSchema }),
  z.object({ type: z.literal("approval.pending"), chatId: id, messageId: id }),
  z.object({ type: z.literal("schedule.updated"), scheduleId: id }),
]) satisfies z.ZodType<UserEvent>;

// Route bodies: the envelope of each success response.

/** `GET` and `PATCH /settings` */
export const settingsBodySchema = detailsResponseSchema(settingsResponseSchema);
/** `GET /providers` */
export const providerListBodySchema = listResponseSchema(providerResponseSchema);
/** `POST /providers`, `PATCH /providers/{id}` */
export const providerBodySchema = detailsResponseSchema(providerResponseSchema);
/** `GET /providers/{id}` */
export const providerDetailsBodySchema = detailsResponseSchema(providerDetailsResponseSchema);
/** `POST /providers/{id}/refresh` */
export const refreshBodySchema = detailsResponseSchema(refreshResponseSchema);
/** `GET /models`, `GET /providers/{id}/models` */
export const modelListBodySchema = listResponseSchema(modelResponseSchema);
/** `PATCH /models/{id}` */
export const modelBodySchema = detailsResponseSchema(modelResponseSchema);
/** `GET /projects/{id}/files` */
export const fileListBodySchema = listResponseSchema(fileResponseSchema);
/** `POST /projects/{id}/files`, `POST /chats/{id}/files`, `PATCH /files/{id}`, `POST /files/{id}/move` */
export const fileBodySchema = detailsResponseSchema(fileResponseSchema);
/** `GET /chats` */
export const chatPageBodySchema = pageResponseSchema(chatSummaryResponseSchema);
/** `POST /chats`, `GET` and `PATCH /chats/{id}` */
export const chatBodySchema = detailsResponseSchema(chatResponseSchema);
/** `GET /chats/{id}/messages` */
export const messageListBodySchema = listResponseSchema(messageResponseSchema);
/** `GET /search` */
export const searchListBodySchema = listResponseSchema(searchHitResponseSchema);
/** `GET /projects` */
export const projectListBodySchema = listResponseSchema(projectResponseSchema);
/** `POST /projects`, `PATCH /projects/{id}`, `POST /chats/{id}/save-as-project` and `/save-instructions` */
export const projectBodySchema = detailsResponseSchema(projectResponseSchema);
/** `GET /projects/{id}` */
export const projectDetailsBodySchema = detailsResponseSchema(projectDetailsResponseSchema);
/** `GET /tool-servers` */
export const toolServerListBodySchema = listResponseSchema(toolServerResponseSchema);
/** `GET`, `PATCH /tool-servers/{id}`, `POST /tool-servers`, `POST /tool-servers/{id}/review` */
export const toolServerBodySchema = detailsResponseSchema(toolServerResponseSchema);
/** `POST /tool-servers/{id}/test` */
export const toolTestBodySchema = detailsResponseSchema(toolTestResponseSchema);
/** `GET /schedules` */
export const scheduleListBodySchema = listResponseSchema(scheduleResponseSchema);
/** `GET`, `PATCH /schedules/{id}`, `POST /schedules` */
export const scheduleBodySchema = detailsResponseSchema(scheduleResponseSchema);
/** `POST /schedules/preview` */
export const schedulePreviewBodySchema = detailsResponseSchema(schedulePreviewResponseSchema);
/** `POST /schedules/{id}/run` */
export const scheduleRunBodySchema = detailsResponseSchema(scheduleRunResponseSchema);
