// Native builds (Tauri) send API calls through `@tauri-apps/plugin-http`, which runs them from
// Rust: no CORS, no mixed-content block, and no Android cleartext limit for a LAN server, while
// bodies still stream. Uploads go through it too, without byte progress (`XMLHttpRequest` would
// meet the limits the plugin avoids). It is imported on demand, so web bundles never load it.

import { setTransport } from "@web/api/client";
import { isNative } from "@web/config";

/** Installs the plugin's `fetch` in native builds; does nothing on the web. */
export async function installNativeTransport(): Promise<void> {
  if (!isNative()) return;
  const http = await import("@tauri-apps/plugin-http");
  setTransport({ fetch: (input, init) => http.fetch(input, init), upload: null });
}
