# Document Titles (Web)

Browser tab titles use **`Copilot | <Action>`**. This rule applies to the web app document title, not Markdown headings (see the `markdown-title-case` rule).

## Component

Use [`PageTitle`](../../apps/web/src/components/Layout/PageTitle.tsx) from `@web/components/Layout/PageTitle`. React hoists the rendered `<title>` into `<head>`:

```tsx
import { PageTitle } from "@web/components/Layout/PageTitle";

export default function WidgetsPage() {
  return (
    <>
      <PageTitle page="Widgets" />
      {/* ... */}
    </>
  );
}
```

- **Home / default**: `<PageTitle />` renders plain `Copilot` (matches `html.title` in `apps/web/rsbuild.config.ts`).
- **Action**: a short page label in **title case** (`Page Not Found`, `Widget Details`).
- **Named items**: a page for one item the user named shows that name as typed, so tabs and history entries tell items apart (a chat's title, a project's name), with a fixed label (`Chat`, `Project`) while it loads.
- **Waiting for the user**: `attention` puts `(!)` before the title while the page needs the user (a tool approval), so the tab stands out in the background.
- Do not set `document.title` in effects.

```tsx
// BAD
<PageTitle page="page not found" />
useEffect(() => { document.title = "Widgets"; }, []);

// GOOD
<PageTitle page="Page Not Found" />
```
