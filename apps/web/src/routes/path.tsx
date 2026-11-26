// Every screen's path. The route table in `routes/index.tsx` maps each one to its page; links use
// `link` to fill in ids. This list is frozen with Phase 1: later steps fill in the pages.

const path = {
  /** New chat (the default screen). */
  home: "/",
  chat: "/chats/:chatId",
  search: "/search",
  scheduled: "/scheduled",
  scheduleNew: "/scheduled/new",
  schedule: "/scheduled/:scheduleId",
  projects: "/projects",
  projectNew: "/projects/new",
  project: "/projects/:projectId",

  settings: "/settings",
  account: "/settings/account",
  providers: "/settings/providers",
  providerNew: "/settings/providers/new",
  provider: "/settings/providers/:providerId",
  toolServers: "/settings/tools",
  toolServerNew: "/settings/tools/new",
  toolServer: "/settings/tools/:serverId",
  users: "/settings/users",
  userNew: "/settings/users/new",
  user: "/settings/users/:userId",

  /** Sign in. */
  login: "/login",
  /** Create an account while registration is open. */
  register: "/register",
  /** Create the first (administrator) account with the setup token. */
  setup: "/setup",
  /** First-run provider setup, after the first sign-in. */
  welcome: "/welcome",
  /** Native builds: choose the server. */
  connect: "/connect",

  notFound: "*",
} as const;

const id = (value: string) => encodeURIComponent(value);

/** Paths with their ids filled in. */
export const link = {
  chat: (chatId: string) => `/chats/${id(chatId)}`,
  schedule: (scheduleId: string) => `/scheduled/${id(scheduleId)}`,
  project: (projectId: string) => `/projects/${id(projectId)}`,
  provider: (providerId: string) => `/settings/providers/${id(providerId)}`,
  toolServer: (serverId: string) => `/settings/tools/${id(serverId)}`,
  user: (userId: string) => `/settings/users/${id(userId)}`,
} as const;

export default path;
