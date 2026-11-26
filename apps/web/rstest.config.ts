import { pluginReact } from "@rsbuild/plugin-react";
import { defineConfig } from "@rstest/core";

// Tests run in Node (fetch, streams, and React's server renderer need no DOM). The React plugin
// compiles JSX the way the app build does.
export default defineConfig({
  plugins: [pluginReact()],
});
