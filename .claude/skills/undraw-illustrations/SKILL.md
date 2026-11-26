---
name: undraw-illustrations
description: Sources, downloads, and applies unDraw.co SVG illustrations with brand coloring for apps/web status pages, empty states, and error views. Use when building pages that need illustrations, adding 404/error/empty-state art, auditing the web app for missing illustrations, or recoloring existing SVG assets in apps/web/src/assets/images/.
disable-model-invocation: true
---

# unDraw Illustrations (apps/web)

Source illustrations from [unDraw](https://undraw.co/illustrations), apply brand coloring, save to the repo asset folder, and wire into status/error/loading views. Reuse existing assets before downloading new ones.

## When to Use

- User invokes this skill or asks for an illustration, empty-state art, or error-page graphic
- Building a new page or feature that needs status/empty/error visuals
- Auditing apps/web for pages missing illustrations or SVGs with wrong brand color
- Recoloring existing illustration SVGs to match brand accent

## `apps/web` Conventions (Fixed)

| Item | Path / value |
|------|----------------|
| Asset folder | `apps/web/src/assets/images/` |
| Layout shell | `apps/web/src/components/Layout/CenteredPage.tsx` |
| Import alias | `@web/assets/images/<name>.svg` |
| File naming | kebab-case, scenario-based (`page-not-found.svg`); existing files may use legacy names (`404.svg`, `generic_error.svg`) |
| Default brand accent | `#3b82f6` (Tailwind `blue-500`, matches `Button` and loading dots) |
| Rendering | `<img src={importedUrl}>`: colors must be baked into the SVG |

Do not use SVGR, `currentColor`, or runtime CSS to theme illustrations.

## Invocation Modes

### Mode A: Feature Scope

Use when the skill is attached while building a specific feature.

1. List UI surfaces in the feature that need art: 404, load error, empty state, loading, under-construction, success
2. Check `apps/web/src/assets/images/` for a reusable file before downloading
3. For gaps: search unDraw, download, save with correct filename, wire import + `<img>` (or extend `ErrorView` with an optional `illustration` prop)
4. Confirm every status/empty/error branch in the feature has an illustration before finishing

### Mode B: Repo Scope

Use when auditing the entire apps/web package.

1. Grep for `ErrorView`, `CenteredPage`, and empty-state branches lacking illustration `<img>` elements
2. Grep routes for error/404/empty UI that is text-only and would benefit from art
3. Audit all SVGs in `apps/web/src/assets/images/`: recolor any using `#6c63ff` or a stale brand hex
4. Add missing illustrations and wire them up
5. End with the mandatory report (below)

For the full audit checklist, see [reference.md](reference.md).

## Workflow

```
Task progress:
- [ ] 1. Apply apps/web illustration conventions
- [ ] 2. Resolve brand accent color
- [ ] 3. Search unDraw for matching illustration
- [ ] 4. Download + apply brand color + save with correct filename
- [ ] 5. Wire into component(s)
- [ ] 6. Verify imports and usage
```

### Step 1: Conventions

Use the fixed paths in the table above. No project detection needed.

### Step 2: Brand Accent Color

Priority order:

1. Grep existing SVGs in `apps/web/src/assets/images/` for the repeated accent `fill` (default: `#3b82f6`)
2. Check `Button` variant classes (`bg-blue-500`) and `DefaultLoading` dot color
3. If ambiguous, or the illustration sits on a tinted background, **ask the user** which hex to use

Brand color is not always the site primary: it may be a contrasting accent that looks good on the illustration's container. When unsure, ask.

### Step 3: Pick the Right Illustration

Search unDraw with semantic keywords from [reference.md](reference.md). Prefer illustrations whose `title` closely matches the scenario (e.g. "Page Not Found" for 404, not a generic "Lost").

### Step 4: Download and Save

unDraw has no official API. Use `undraw-cli` for machine-readable search and download:

```bash
# Search (parse JSON output)
pnpm exec undraw-cli list "page not found" --json

# Download with brand color
pnpm exec undraw-cli download <id> --color "#3b82f6" --json --out /tmp

# Rename to apps/web convention (drop upstream slug suffixes)
mv /tmp/<upstream-name>.svg apps/web/src/assets/images/page-not-found.svg
```

**CLI fallback chain:** `pnpm exec undraw-cli` → `npx undraw-cli` → `pip install undraw-py` + `undraw download` → manual browser download from undraw.co + replace `#6c63ff` in file → ask user for a mirror CDN.

If the user provides a mirror (e.g. `https://undraw.oswaldlabs.com/{hex}/{name}.svg`), fetch from there and save locally.

### Step 5: Wire Up

Follow existing patterns in `NotFoundPage`, `ErrorView`, and `DefaultLoading`:

```typescript
import pageNotFoundIllustration from "@web/assets/images/page-not-found.svg";
import { CenteredPage } from "@web/components/Layout/CenteredPage";

<CenteredPage>
  <img
    src={pageNotFoundIllustration}
    alt="Illustration of a page not found error"
    className="mx-auto mb-6 h-96 w-auto"
  />
  {/* title, message, actions */}
</CenteredPage>
```

Common wiring patterns:

- Global 404: `apps/web/src/routes/not-found/page.tsx`
- Generic error: `apps/web/src/components/Error/ErrorView.tsx` (hardcoded illustration today; add optional prop if needed)
- Error boundary: `apps/web/src/components/Error/ErrorBoundary.tsx` → `ErrorView`
- Loading: `apps/web/src/components/Suspense/DefaultLoading.tsx`
- Route error page: `apps/web/src/routes/error/page.tsx`

### Step 6: Recolor Existing SVGs

Grep `apps/web/src/assets/images/` for `#6c63ff` or wrong accent hex. Replace accent fills only: preserve structural colors (`#3f3d56`, `#2f2e41`, skin tones, `#f2f2f2` backgrounds). See [reference.md](reference.md).

## Mandatory Report Template

Every run ends with:

```markdown
## Illustration pass complete

**Scope:** [feature area | full repo]
**Brand accent:** [#hex and how it was determined]

### Added
- [filename.svg → component/page] or "None"

### Recolored
- [filename.svg: old → new hex] or "None"

### Reused (no download)
- [existing file → where used] or "None"

### Skipped / needs user input
- [page + reason] or "None"
```

## Additional Resources

- Scenario keywords, color rules, audit checklist: [reference.md](reference.md)
- Before/after examples: [examples.md](examples.md)
