// TanStack Query keys of the settings screens. Keys are prefixes of each other where one change
// makes several answers stale: invalidating `providers` also refreshes every connection's details
// and model list.

export const keys = {
  sessions: ["auth", "sessions"] as const,
  settings: ["settings"] as const,

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
} as const;
