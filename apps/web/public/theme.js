// Applies the stored color scheme before the first paint, so a pinned theme never flashes the
// system one (src/theme.ts owns the setting). A file rather than an inline script, so the strict
// Content Security Policy (script-src 'self') allows it. Without it the page follows the system.
try {
  const scheme = localStorage.getItem("copilot.color-scheme");
  const meta = document.querySelector('meta[name="color-scheme"]');
  if (meta && (scheme === "light" || scheme === "dark")) meta.content = scheme;
} catch {
  // Storage is blocked: follow the system scheme.
}
