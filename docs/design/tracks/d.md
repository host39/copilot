# Track D: Web

Branch: `claude/copilot-project-planning-6y3fep` (shared with the other track checkout; merged before each push). Base: `p1-frozen`. Roadmap step: P2.D. Owns `apps/web/src/routes/**`, `apps/web/src/components/**`, and `apps/web/src/api/routes/**`, plus new test files under `apps/web/test/`.

## Status

Part 1 done (P2.D.1 to P2.D.5): Settings and the first-run setup. Part 2 (projects and files, sidebar and chat list, search, the non-streaming chat view) is open. Last green CI: `b917908` (Code quality, run 45918865624), which holds every part 1 commit.

## Sub-Steps

Part 1: Settings (account, devices, providers and models, tool servers, users) and the first-run setup. Part 2: projects and files with the capacity bar, sidebar and chat list, search, non-streaming chat view.

- [x] P2.D.1 Request helpers for every Settings route (`api/routes/auth.ts` account and sessions, `settings.ts`, `providers.ts`, `models.ts`, `tool-servers.ts`, `users.ts`, query keys in `keys.ts`); shared Settings components (`SettingsPage`, `Section`, `QueryState`, `ConfirmDialog`, `Badge`, `Switch`, `TextArea`, `FormNotice`); Settings, Account: display name, password change (other devices signed out), personal instructions, theme, and Devices (this device first, then by last use; sign out one device, this one, or every other one). Tests: `test/settings-api.test.ts` (method, path, query, and body of every helper), `test/account.test.tsx` (password form rules, device names and order, relative times, the screen).
- [x] P2.D.2 Providers and models: the connection list (preset, address, off and unreadable-key badges); Add provider (preset radio group, then the key, the address, or both; a blank name takes the preset's), which saves the connection and tests it at once by listing its models, keeping the connection when the test fails; a connection's page with the Connection form (only changed fields sent, a blank key keeps the stored one, a note when a new origin drops the key), the on/off switch, Remove key, Test and refresh with a summary of what changed, the models (search by every word of name or id, filter on or off, models no longer listed last, 50 at a time, an on/off switch each, an editor for display name, context window, max output tokens, and capability overrides that sends only what changed and never a locked flag), and Delete with the usage counts read again when the confirmation opens. Tests: `test/providers.test.tsx` (form rules per preset, add-and-test with a failed test, refresh summaries, the connection form and key drop, model filter and order, the model editor's rules and diffs, usage sentences, the list, add, and connection screens).
- [x] P2.D.3 Tool servers: the server list (tool count or "Not tested yet", off, Auto, and review badges); Add tool server (name, address, bearer token, custom header rows with hidden values, approval with Ask first and a warning on Auto, an optional slug), which registers the server and tests it at once, keeping it when the test fails; a server's page with Test connection (what was added, removed, or changed), the review banner with Accept changes while changed tools ask, the tool list (title, name, description, a read-only hint that suggests Auto without choosing it) with an Off, Ask, or Auto picker each, saved together as one `toolModes` map of the listed tools; the Server form (only changed fields; a new address asks for a new test), the on/off switch, Credentials (replaced together, removed after a confirmation), and Delete. The credential rules mirror core's `ToolHeaders` (HTTP token names, reserved and `mcp-*` names, duplicates ignoring case, one-line values, bearer or a custom Authorization header, no spaces in the token). Tests: `test/tool-servers.test.tsx` (every credential rule, the add and settings forms, tool modes, test summaries, add-and-test with a failed test, the list, add, and server screens).
- [x] P2.D.4 Users (administrators only, on rustauth's `/admin/users*` routes): the account list, 100 per page with Previous and Next, the caller marked "You", role, disabled, and locked badges; Add user (name, username, a temporary password typed or generated from the browser's cryptographic random source, the role with what each allows), which shows the password once afterwards to hand over instead of keeping it in history state; an account's page with the profile (name and role, only changes sent), access (disable after a confirmation, enable, unlock while locked), a password reset (which also ends a lockout), and delete (after a confirmation that the account's data goes with it). Every change signs that account out everywhere, so the caller's own account carries a warning; the server still refuses to leave no enabled administrator and the screens show its message. Tests: `test/users.test.tsx` (lockouts, badges, temporary passwords and their uniform sampling, the profile and reset forms with rustauth's rules, the list with paging, add, an account, and the caller's own account).
- [x] P2.D.5 First-run setup (`/welcome`) on the provider routes, in three steps: (1) a preset and its key or address saves the connection; (2) Test lists its models at once and says what it found, or why it failed, with Back to fix the key or address (which updates the saved connection, or replaces it when the preset changed, so a retry never adds a second one) and Test again; (3) the models, all turned on by the server when it lists 20 or fewer, otherwise picked with search; Start chatting opens the new chat screen with the first model turned on. A sign-in lands in the setup while the user has no provider connection (registration and setup already went there), and the new chat screen without models links to it ("Add a provider") and to Settings. Tests: `test/welcome.test.tsx` (the step reducer, keeping a saved key, saving, fixing, and replacing the connection, the model the chat opens with, where a sign-in lands, the setup and new chat screens).

Part 2:

- [ ] P2.D.6 Projects and files with the capacity bar.
- [ ] P2.D.7 Sidebar and chat list.
- [ ] P2.D.8 Search.
- [ ] P2.D.9 Non-streaming chat view.

## Notes

- Settings screens load with TanStack Query (keys in `api/routes/keys.ts`, prefixes of each other so one invalidation refreshes a connection's details and models together) and submit forms through `useSubmit`, so the request schemas from `@copilot/contracts` check input before sending and a rejected field shows next to its input.
- Each Settings entity has one page module for both its `new` and `:id` paths (the frozen route table maps both to it): `page.tsx` picks `add.tsx` or `details.tsx` by the path parameter.
- Adding a provider or a tool server saves it and tests it in one action; a failed test keeps it, and its page shows why (location state carries the outcome, never a secret). The tool server test answers `502 ADAPTER_UNAVAILABLE` until P2.F wires the MCP adapter; the screens show that message.
- Later users see the first-run setup on sign-in while they have no provider connection, and again on later sign-ins until they connect one (Skip for now goes to the new chat screen). The decision lives in `routes/welcome/setup.ts` (`landing`), which the sign-in screen calls; a failed check signs in as before.
- A new account's temporary password is shown once on the add screen after creating it, not passed through history state.
- Outside the owned paths: `apps/web/src/assets/styles/main.css` gained `.button-link`, `.list-panel`, `.list-row`, and `.form-warning` (shared-css rule), and `apps/web/test/chat.test.tsx` changed its one test of the new chat screen without models (it now links to the setup too). The new chat screen (`routes/home/page.tsx`) reads the setup's model from location state.
- Switches are controlled checkboxes with `role="switch"` and `aria-checked`; confirmations are a modal `<dialog>` whose Cancel button comes first, so it takes the initial focus.

## Blockers

None.

## Interface-Change Requests

None.
