// Every Copilot API route, relative to `/api` (see the spec's HTTP API section). rustauth's routes
// (`/auth/*`, `/admin/users*`) come from `authRoutes` in `@copilot/contracts`. `/health` and
// `/health/ready` live at the server root: call them with `{ root: true }`.

import { authRoutes } from "@copilot/contracts";

const id = (value: string) => encodeURIComponent(value);

const API = {
  auth: authRoutes,

  health: "/health",
  ready: "/health/ready",

  settings: "/settings",
  capabilities: "/capabilities",

  providers: "/providers",
  provider: (providerId: string) => `/providers/${id(providerId)}`,
  providerRefresh: (providerId: string) => `/providers/${id(providerId)}/refresh`,
  providerModels: (providerId: string) => `/providers/${id(providerId)}/models`,

  models: "/models",
  model: (modelId: string) => `/models/${id(modelId)}`,

  projects: "/projects",
  project: (projectId: string) => `/projects/${id(projectId)}`,
  projectFiles: (projectId: string) => `/projects/${id(projectId)}/files`,

  file: (fileId: string) => `/files/${id(fileId)}`,
  fileMove: (fileId: string) => `/files/${id(fileId)}/move`,
  fileContent: (fileId: string) => `/files/${id(fileId)}/content`,

  chats: "/chats",
  chat: (chatId: string) => `/chats/${id(chatId)}`,
  chatRead: (chatId: string) => `/chats/${id(chatId)}/read`,
  chatFiles: (chatId: string) => `/chats/${id(chatId)}/files`,
  chatMessages: (chatId: string) => `/chats/${id(chatId)}/messages`,
  chatRegenerate: (chatId: string) => `/chats/${id(chatId)}/regenerate`,
  chatRetry: (chatId: string) => `/chats/${id(chatId)}/retry`,
  chatDecisions: (chatId: string, messageId: string) =>
    `/chats/${id(chatId)}/messages/${id(messageId)}/decisions`,
  chatStream: (chatId: string) => `/chats/${id(chatId)}/stream`,
  chatStop: (chatId: string) => `/chats/${id(chatId)}/stop`,
  chatSaveAsProject: (chatId: string) => `/chats/${id(chatId)}/save-as-project`,
  chatSaveInstructions: (chatId: string) => `/chats/${id(chatId)}/save-instructions`,

  search: "/search",

  toolServers: "/tool-servers",
  toolServer: (serverId: string) => `/tool-servers/${id(serverId)}`,
  toolServerTest: (serverId: string) => `/tool-servers/${id(serverId)}/test`,
  toolServerReview: (serverId: string) => `/tool-servers/${id(serverId)}/review`,
  toolServerSignIn: (serverId: string) => `/tool-servers/${id(serverId)}/oauth/start`,
  toolServerSignOut: (serverId: string) => `/tool-servers/${id(serverId)}/oauth`,

  schedules: "/schedules",
  schedule: (scheduleId: string) => `/schedules/${id(scheduleId)}`,
  scheduleRun: (scheduleId: string) => `/schedules/${id(scheduleId)}/run`,
  schedulePreview: "/schedules/preview",

  events: "/events",
} as const;

export default API;
