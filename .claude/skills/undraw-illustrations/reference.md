# unDraw Illustrations: Reference

## Scenario → Search Keywords

Use these keywords with `pnpm exec undraw-cli list "<keyword>" --json`. Pick the result whose `title` best matches the UI scenario.

| Scenario | Search keywords | Preferred filename | Existing asset |
|----------|----------------|-------------------|----------------|
| 404 / not found | `page not found`, `404` | `page-not-found.svg` | Yes (`404.svg`) |
| Server / 5xx error | `server down`, `server error` | `server-down.svg` | No |
| Network / load failure | `connection lost`, `no signal`, `offline` | `connection-lost.svg` | No |
| Generic warning / app error | `warning`, `alert`, `something went wrong` | `generic-error.svg` | Yes (`generic_error.svg`) |
| Loading / in progress | `loading`, `progress` | `loading.svg` | Yes |
| Empty list / no data | `empty`, `void`, `no data`, `nothing here` | `empty.svg` | No |
| No items of a type | feature-specific (e.g. `credit card`) | `<feature>-empty.svg` | No |
| All done / no jobs | `completed`, `tasks done`, `checklist` | `completed-tasks.svg` | No |
| Under construction / coming soon | `under construction`, `building` | `under-construction.svg` | No |
| Success / celebration | `celebration`, `success`, `congratulations` | `celebration.svg` | No |
| Unauthorized / access denied | `access denied`, `unauthorized` | `access-denied.svg` | No |
| Search no results | `no results`, `search` | `no-results.svg` | No |

When a scenario-specific file already exists, **reuse it** across pages rather than downloading duplicates. Legacy filenames (`404.svg`, `generic_error.svg`) may remain until a deliberate rename pass.

## Filename Rules

- **kebab-case**, scenario-based: `page-not-found.svg`, not `PageNotFound.svg`
- **No upstream slug suffixes**: drop `_ied1`, `_re_3_0`, hash IDs from unDraw download names
- **Match the UI purpose**, not the unDraw title verbatim: "Page Not Found" → `page-not-found.svg`
- One file per scenario; share via imports, not copies

## Brand Color Rules

### Default Accent

apps/web default: `#3b82f6` (Tailwind `blue-500`, used by `Button` and `DefaultLoading`).

### What to Replace

Replace unDraw's default accent `#6c63ff` with the brand hex. Also replace any stale accent that differs from the current brand (grep for `#6c63ff` and verify against existing illustration files).

### What NOT to Replace

| Color | Role |
|-------|------|
| `#3f3d56`, `#2f2e41`, `#090814` | Dark structural shapes, outlines |
| `#f2f2f2`, `#e6e6e6`, `#e4e4e4`, `#ccc`, `#cacaca` | Backgrounds, neutrals |
| `#ffb8b8`, `#ffb9b9`, `#9e616a` | Skin tones |
| `#fff`, `#ffffff` | White fills |

### Contrast Guidance

Illustrations render inside `CenteredPage` on a white/slate layout. The accent should read clearly against that background. If `#3b82f6` feels too similar to surrounding UI, ask the user for a contrasting accent (e.g. `#7c3aed` purple, `#059669` green).

## CLI Commands

This monorepo uses pnpm: run `undraw-cli` via `pnpm exec` (or `npx` as fallback).

### Undraw-Cli (preferred)

```bash
# List/search with JSON output
pnpm exec undraw-cli list "page not found" --json
pnpm exec undraw-cli list "empty" --json

# Download with brand color
pnpm exec undraw-cli download <id> --color "#3b82f6" --json --out ./tmp
```

JSON `list` response includes `items[]` with `id`, `title`, `svg_url`.
JSON `download` response includes `path`, `color`, `bytes`.

### Undraw-Py (fallback)

```bash
pip install undraw-py
undraw list "page not found" --json
undraw download <id> --color "#3b82f6"
```

### Manual Fallback

1. Browse https://undraw.co/illustrations
2. Download SVG
3. Replace all `#6c63ff` with brand hex in a text editor
4. Save to `apps/web/src/assets/images/<scenario>.svg`

### Mirror CDN (if User Provides)

```
https://undraw.oswaldlabs.com/<hex-without-hash>/<illustration-name>.svg
```

Example: `https://undraw.oswaldlabs.com/3b82f6/page-not-found.svg`

## Apps/web Paths

| Item | Path |
|------|------|
| Illustrations | `apps/web/src/assets/images/` |
| Layout shell | `apps/web/src/components/Layout/CenteredPage.tsx` |
| Generic error | `apps/web/src/components/Error/ErrorView.tsx` |
| Error boundary | `apps/web/src/components/Error/ErrorBoundary.tsx` |
| Global 404 | `apps/web/src/routes/not-found/page.tsx` |
| Route error page | `apps/web/src/routes/error/page.tsx` |
| Loading | `apps/web/src/components/Suspense/DefaultLoading.tsx` |
| Type declarations | `apps/web/src/env.d.ts` (`*.svg` → URL string) |

Import pattern:

```typescript
import fooIllustration from "@web/assets/images/foo.svg";
```

## Repo Audit Checklist

Walk these files when running Mode B (repo scope):

### Error / Status Components

- [ ] `apps/web/src/routes/not-found/page.tsx`: `404.svg`
- [ ] `apps/web/src/routes/error/page.tsx`: uses `ErrorView` (inherits `generic_error.svg`)
- [ ] `apps/web/src/components/Error/ErrorView.tsx`: `generic_error.svg`
- [ ] `apps/web/src/components/Error/ErrorBoundary.tsx`: delegates to `ErrorView`
- [ ] `apps/web/src/components/Suspense/DefaultLoading.tsx`: `loading.svg`

### Routes to Check for Gaps

Scan `apps/web/src/routes/` for:

- Error branches (`if (error)`, `isError`, `catch`) without illustration
- Empty arrays rendered as plain text without `<img>` + `CenteredPage`
- New pages added since last audit

### SVG Color Audit

```bash
grep -l '#6c63ff' apps/web/src/assets/images/*.svg
```

Recolor any matches. Also spot-check that existing files use `#3b82f6` (or the resolved brand accent) as accent.

### Current Illustration Inventory

| File | Used by |
|------|---------|
| `404.svg` | `routes/not-found/page.tsx` |
| `generic_error.svg` | `components/Error/ErrorView.tsx` |
| `loading.svg` | `components/Suspense/DefaultLoading.tsx` |
