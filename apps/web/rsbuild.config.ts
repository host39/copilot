import path from "node:path";
import { defineConfig } from "@rsbuild/core";
import { pluginReact } from "@rsbuild/plugin-react";

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
    },
    tags: [
      // Before the first paint, and not deferred, so a pinned scheme never flashes.
      { tag: "script", attrs: { src: "/theme.js" }, head: true, append: true },
    ],
  },
  server: {
    port: 4000,
  },
  resolve: {
    alias: {
      "@web": path.join(import.meta.dirname, "src"),
      "@contracts": path.join(import.meta.dirname, "../../packages/contracts/ts"),
    },
  },
});
