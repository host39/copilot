import path from "node:path";
import { defineConfig } from "@rsbuild/core";
import { pluginReact } from "@rsbuild/plugin-react";

export default defineConfig({
  plugins: [pluginReact()],
  html: {
    title: "Copilot",
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
