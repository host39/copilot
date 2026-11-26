import path from "node:path";
import { defineConfig } from "@rsbuild/core";
import { pluginReact } from "@rsbuild/plugin-react";

// Where the dev server forwards `/api` and `/health`: the API of this checkout. The web client calls
// its own origin, so development needs no CORS.
const apiTarget = process.env.API_PROXY_TARGET || "http://localhost:3000";

export default defineConfig({
  plugins: [pluginReact()],
  html: {
    title: "Copilot",
    template: "./index.html",
    meta: {
      // `resizes-content` keeps the composer above the on-screen keyboard.
      viewport: "width=device-width, initial-scale=1, interactive-widget=resizes-content",
      // Both schemes; `public/theme.js` narrows it to a scheme the user pinned.
      "color-scheme": "light dark",
      description: "A self-hosted chat client for any model provider.",
      "theme-color-light": {
        name: "theme-color",
        content: "#ffffff",
        media: "(prefers-color-scheme: light)",
      },
      "theme-color-dark": {
        name: "theme-color",
        content: "#1b1c1f",
        media: "(prefers-color-scheme: dark)",
      },
    },
    tags: [
      // Before the first paint, and not deferred, so a pinned scheme never flashes.
      { tag: "script", attrs: { src: "/theme.js" }, head: true, append: true },
      // Installable from the browser (Android, Linux) before the native apps exist.
      { tag: "link", attrs: { rel: "manifest", href: "/manifest.webmanifest" }, head: true },
      { tag: "link", attrs: { rel: "icon", href: "/icons/icon.svg", type: "image/svg+xml" } },
      { tag: "link", attrs: { rel: "icon", href: "/icons/icon-192.png", sizes: "192x192" } },
      { tag: "link", attrs: { rel: "apple-touch-icon", href: "/icons/apple-touch-icon.png" } },
    ],
  },
  server: {
    port: 4000,
    proxy: {
      "/api": apiTarget,
      "/health": apiTarget,
    },
  },
  resolve: {
    alias: {
      "@web": path.join(import.meta.dirname, "src"),
      "@contracts": path.join(import.meta.dirname, "../../packages/contracts/ts"),
    },
  },
});
