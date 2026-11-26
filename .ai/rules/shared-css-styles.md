# Shared CSS Styles (Web)

Do **not** create TypeScript files that export shared Tailwind class strings (for example `export const BUTTON_CLASS = "..."`) for reuse across files.

## When Styles Are Reused

If the same class list appears in **two or more** files, add a utility class to `apps/web/src/assets/styles/main.css` and use `className="utility-name"`:

```css
/* main.css */
.card-panel {
  @apply rounded-xl border border-gray-200 bg-white p-8 shadow-sm;
}
```

```tsx
<div className="card-panel">...</div>
```

## When Inline Classes Are Fine

- One-off layout or page-specific styling
- Component-local maps such as `VARIANT_CLASS_MAP` in `Button.tsx`, used only inside that component
- Dynamic class composition (`isHealthy ? "..." : "..."`) inside a single component

Keep utility names short and semantic (`card-panel`, not `homePageHealthCardContainerClass`).
