# Track D: Web

Branch: `claude/copilot-project-planning-6y3fep` (shared with the other track checkout; merged before each push). Base: `p1-frozen`. Roadmap step: P2.D. Owns `apps/web/src/routes/**`, `apps/web/src/components/**`, and `apps/web/src/api/routes/**`, plus new test files under `apps/web/test/`.

## Status

Part 1 in progress: Settings and the first-run setup. Part 2 (projects and files, sidebar and chat list, search, the non-streaming chat view) is open.

## Sub-Steps

Part 1: Settings (account, devices, providers and models, tool servers, users) and the first-run setup. Part 2: projects and files with the capacity bar, sidebar and chat list, search, non-streaming chat view.

- [x] P2.D.1 Request helpers for every Settings route (`api/routes/auth.ts` account and sessions, `settings.ts`, `providers.ts`, `models.ts`, `tool-servers.ts`, `users.ts`, query keys in `keys.ts`); shared Settings components (`SettingsPage`, `Section`, `QueryState`, `ConfirmDialog`, `Badge`, `Switch`, `TextArea`, `FormNotice`); Settings, Account: display name, password change (other devices signed out), personal instructions, theme, and Devices (this device first, then by last use; sign out one device, this one, or every other one). Tests: `test/settings-api.test.ts` (method, path, query, and body of every helper), `test/account.test.tsx` (password form rules, device names and order, relative times, the screen).
- [ ] P2.D.2 Providers and models.
- [ ] P2.D.3 Tool servers.
- [ ] P2.D.4 Users (administrators only).
- [ ] P2.D.5 First-run setup on the provider routes.

## Notes

- Settings screens load with TanStack Query (keys in `api/routes/keys.ts`, prefixes of each other so one invalidation refreshes a connection's details and models together) and submit forms through `useSubmit`, so the request schemas from `@copilot/contracts` check input before sending and a rejected field shows next to its input.
- Switches are controlled checkboxes with `role="switch"` and `aria-checked`; confirmations are a modal `<dialog>` whose Cancel button comes first, so it takes the initial focus.

## Blockers

None.

## Interface-Change Requests

None.
