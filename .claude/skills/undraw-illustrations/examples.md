# unDraw Illustrations: Examples

## 1. New 404 Page

**Scenario:** Adding a feature-specific not-found view.

**Search:**

```bash
pnpm exec undraw-cli list "page not found" --json
```

Pick an item with title close to "Page Not Found".

**Download and save:**

```bash
pnpm exec undraw-cli download page_not_found --color "#3b82f6" --json --out /tmp
mv /tmp/page_not_found.svg apps/web/src/assets/images/page-not-found.svg
```

**Wire up** (reuse existing pattern from `not-found/page.tsx`):

```typescript
import notFoundIllustration from "@web/assets/images/page-not-found.svg";
import Button from "@web/components/Button/Button";
import { CenteredPage } from "@web/components/Layout/CenteredPage";

<CenteredPage>
  <img
    src={notFoundIllustration}
    alt="Illustration of a page not found error"
    className="mx-auto mb-6 h-96 w-auto"
  />
  <p className="text-sm font-medium uppercase tracking-[0.2em] text-slate-400">404</p>
  <h1 className="mt-2 text-2xl font-semibold tracking-tight text-slate-900 sm:text-3xl">
    Page not found
  </h1>
  {/* message + Button actions */}
</CenteredPage>
```

**Report entry:**

```
### Reused (no download)
- 404.svg → not-found/page.tsx (existing asset)
```

Or if a new file was needed:

```
### Added
- page-not-found.svg → FeatureNotFoundPage
```

---

## 2. Empty State for a New Feature

**Scenario:** New route shows a list; when empty, needs art.

**Search:**

```bash
pnpm exec undraw-cli list "no data" --json
pnpm exec undraw-cli list "investing" --json
```

If a feature-specific illustration fits better than a generic empty state, download with a descriptive name:

```bash
pnpm exec undraw-cli download investing_re_3_0 --color "#3b82f6" --json --out /tmp
mv /tmp/investing_re_3_0.svg apps/web/src/assets/images/investments-empty.svg
```

**Wire up** in the route page:

```typescript
import investmentsEmptyIllustration from "@web/assets/images/investments-empty.svg";
import { CenteredPage } from "@web/components/Layout/CenteredPage";

<CenteredPage>
  <img
    src={investmentsEmptyIllustration}
    alt="No investments yet"
    className="mx-auto mb-6 h-96 w-auto"
  />
  <h1 className="mt-2 text-2xl font-semibold tracking-tight text-slate-900 sm:text-3xl">
    No investments yet
  </h1>
  <p className="mt-3 text-sm text-slate-600 sm:text-base">
    Add an investment account to get started.
  </p>
</CenteredPage>
```

If a shared `empty.svg` is sufficient, skip the download:

```
### Reused (no download)
- empty.svg → InvestmentsPage (shared empty-state asset)
```

---

## 3. Repo Audit: Recolor Stale SVG

**Scenario:** Mode B audit finds `generic_error.svg` still contains `#6c63ff`.

**Detect:**

```bash
grep '#6c63ff' apps/web/src/assets/images/generic_error.svg
```

**Fix:** Replace `#6c63ff` → `#3b82f6` in the file (accent only; leave `#3f3d56` etc. untouched).

Alternatively re-download:

```bash
pnpm exec undraw-cli download warning --color "#3b82f6" --json --out /tmp
mv /tmp/warning.svg apps/web/src/assets/images/generic_error.svg
```

**Report entry:**

```
### Recolored
- generic_error.svg: #6c63ff → #3b82f6
```

---

## 4. Ambiguous Brand Color

**Scenario:** New illustration for a page with a dark-themed container, not the default white/slate layout.

**Action:** Do not assume `#3b82f6`. Ask the user:

> The illustration will sit on a `bg-slate-900` container. Should I use the default brand accent `#3b82f6`, or a contrasting color for better visibility?

**If user picks `#f59e0b` (amber):**

```bash
pnpm exec undraw-cli download warning --color "#f59e0b" --json --out /tmp
mv /tmp/warning.svg apps/web/src/assets/images/maintenance.svg
```

**Report entry:**

```
**Brand accent:** #f59e0b (user chose contrasting amber for dark container)

### Added
- maintenance.svg → MaintenancePage
```
