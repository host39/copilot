// Light and dark themes. The page follows the system scheme by default; the user can pin one in
// Settings, Account. The choice lives in `localStorage` and in `<meta name="color-scheme">`, which
// `main.css` maps to the `color-scheme` property that every color token (`light-dark()`) follows.
// `public/theme.js` applies a stored choice before the first paint.

import { readItem, removeItem, writeItem } from "@web/storage";

export type Theme = "system" | "light" | "dark";

export const THEMES: readonly Theme[] = ["system", "light", "dark"];

const THEME_KEY = "copilot.color-scheme";

/** The stored choice; anything unknown reads as `system`. */
export function readTheme(): Theme {
  const stored = readItem(THEME_KEY);
  return stored === "light" || stored === "dark" ? stored : "system";
}

/** The `color-scheme` value for a choice. */
export function colorScheme(theme: Theme): string {
  return theme === "system" ? "light dark" : theme;
}

/** Shows `theme` on this page. */
export function applyTheme(theme: Theme): void {
  const meta = globalThis.document?.querySelector<HTMLMetaElement>('meta[name="color-scheme"]');
  if (meta) meta.content = colorScheme(theme);
}

/** Stores and shows `theme`; `system` removes the stored choice. */
export function saveTheme(theme: Theme): void {
  if (theme === "system") removeItem(THEME_KEY);
  else writeItem(THEME_KEY, theme);
  applyTheme(theme);
}

// Another tab changed the theme: show the same one.
globalThis.addEventListener?.("storage", (event: StorageEvent) => {
  if (event.key === THEME_KEY || event.key === null) applyTheme(readTheme());
});
