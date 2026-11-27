// TanStack Query keys. Keys are prefixes of each other where one change makes several answers
// stale: invalidating `providers` also refreshes every connection's details and model list, and
// invalidating `chats` refreshes the sidebar's lists together with every open chat.

import type { ChatListQuery, ProjectListQuery, ScheduleListQuery } from "@copilot/contracts";

/** The filters of a chat list (`GET /chats` without its cursor). */
export type ChatFilter = Omit<ChatListQuery, "cursor">;

export const keys = {
  sessions: ["auth", "sessions"] as const,
  settings: ["settings"] as const,
  /** What the server lets the caller do (`GET /capabilities`). */
  capabilities: ["capabilities"] as const,

  providers: ["providers"] as const,
  provider: (providerId: string) => ["providers", providerId] as const,
  providerModels: (providerId: string) => ["providers", providerId, "models"] as const,
  /** The model picker (`GET /models`). */
  models: ["models"] as const,

  toolServers: ["tool-servers"] as const,
  toolServer: (serverId: string) => ["tool-servers", serverId] as const,

  users: ["admin", "users"] as const,
  usersPage: (offset: number) => ["admin", "users", "page", offset] as const,
  user: (userId: string) => ["admin", "users", userId] as const,

  projects: ["projects"] as const,
  projectList: (query: ProjectListQuery) => ["projects", "list", query] as const,
  project: (projectId: string) => ["projects", projectId] as const,
  projectFiles: (projectId: string) => ["projects", projectId, "files"] as const,

  chats: ["chats"] as const,
  /** The pages of one chat list (an infinite query). Chat ids are UUIDs, never `list`, `runs`, or `latest`. */
  chatList: (filter: ChatFilter) => ["chats", "list", filter] as const,
  /** The latest schedule runs, for the sidebar's unread count. */
  runs: ["chats", "runs"] as const,
  /** The most recently updated chat, whose model a new chat starts with. */
  latestChat: ["chats", "latest"] as const,
  chat: (chatId: string) => ["chats", chatId] as const,
  chatMessages: (chatId: string) => ["chats", chatId, "messages"] as const,

  search: (text: string) => ["search", text] as const,

  schedules: ["schedules"] as const,
  /** Schedule ids are UUIDs, never `list` or `preview`. */
  scheduleList: (query: ScheduleListQuery) => ["schedules", "list", query] as const,
  schedule: (scheduleId: string) => ["schedules", scheduleId] as const,
  schedulePreview: (cron: string, timezone: string) =>
    ["schedules", "preview", cron, timezone] as const,
} as const;
