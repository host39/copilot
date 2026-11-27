// The native shell's configuration: what it bundles, its Content-Security-Policy (the web app's
// policy with API traffic on IPC), and the capability that scopes every plugin it ships.

import { readFileSync } from "node:fs";
import { describe, expect, test } from "@rstest/core";

const read = (file: string) => readFileSync(file, "utf8");

type Permission = string | { identifier: string; allow?: { url: string }[] };

const config = JSON.parse(read("src-tauri/tauri.conf.json")) as {
  productName: string;
  version: string;
  identifier: string;
  build: Record<string, unknown>;
  app: {
    withGlobalTauri: boolean;
    windows: Record<string, unknown>[];
    security: Record<string, unknown> & { csp: string };
  };
};

const capability = JSON.parse(read("src-tauri/capabilities/default.json")) as {
  windows: string[];
  permissions: Permission[];
};

/** A policy as its directives, each with its sources in order. */
function directives(policy: string): Map<string, string[]> {
  const parsed = new Map<string, string[]>();
  for (const part of policy.split(";")) {
    const [name, ...sources] = part.trim().split(/\s+/);
    if (name) parsed.set(name, sources);
  }
  return parsed;
}

/** The policy the server sends with the web bundle (`CSP` in `apps/api/src/routes/web.rs`). */
function webPolicy(): string {
  const source = read("../api/src/routes/web.rs");
  const literal = /pub const CSP: &str = "((?:[^"\\]|\\\n|\\.)*)";/.exec(source)?.[1];
  if (literal === undefined) throw new Error("CSP not found in web.rs");
  return literal.replace(/\\\n\s*/g, "");
}

function permission(identifier: string): Permission | undefined {
  return capability.permissions.find(
    (entry) => (typeof entry === "string" ? entry : entry.identifier) === identifier
  );
}

function scope(identifier: string): string[] {
  const entry = permission(identifier);
  return typeof entry === "object" ? (entry.allow ?? []).map((rule) => rule.url) : [];
}

describe("app", () => {
  test("is Copilot under the identifier com.host39.copilot, versioned by package.json", () => {
    expect(config.productName).toBe("Copilot");
    expect(config.identifier).toBe("com.host39.copilot");
    expect(config.version).toBe("../package.json");
    expect(JSON.parse(read("package.json")).version).toMatch(/^\d+\.\d+\.\d+$/);
  });

  test("bundles the web app's production build and develops against its dev server", () => {
    expect(config.build).toMatchObject({
      beforeDevCommand: "pnpm --filter @copilot/web dev",
      devUrl: "http://localhost:4000",
      beforeBuildCommand: "pnpm --filter @copilot/web build",
      frontendDist: "../../web/dist",
    });
  });

  test("keeps the default http scheme, so localStorage (server URL and token) never moves", () => {
    for (const window of config.app.windows) expect(window.useHttpsScheme ?? false).toBe(false);
    expect(config.app.withGlobalTauri).toBe(false);
  });

  test("the Android project uses the same identifier for its Kotlin package and application id", () => {
    const gradle = read("src-tauri/gen/android/app/build.gradle.kts");
    expect(gradle).toContain('namespace = "com.host39.copilot"');
    expect(gradle).toContain('applicationId = "com.host39.copilot"');
    expect(
      read("src-tauri/gen/android/app/src/main/java/com/host39/copilot/MainActivity.kt")
    ).toMatch(/^package com\.host39\.copilot$/m);
  });
});

describe("android", () => {
  const gradle = read("src-tauri/gen/android/app/build.gradle.kts");
  const activity = read(
    "src-tauri/gen/android/app/src/main/java/com/host39/copilot/MainActivity.kt"
  );

  test("release APKs are signed with the debug key, so they install by sideloading", () => {
    // The release block is the last build type, so it runs to the end of `buildTypes`.
    const release = gradle.split('getByName("release")')[1]?.split("compileOptions")[0] ?? "";
    expect(release).toContain('signingConfig = signingConfigs.getByName("debug")');
  });

  test("release APKs keep cleartext off in the WebView; API calls go through the plugin", () => {
    expect(gradle).toContain('manifestPlaceholders["usesCleartextTraffic"] = "false"');
    const release = gradle.split('getByName("release")')[1]?.split("compileOptions")[0] ?? "";
    expect(release).not.toContain("usesCleartextTraffic");
  });

  test("asks only for the network and the microphone, and never for the camera", () => {
    const manifest = read("src-tauri/gen/android/app/src/main/AndroidManifest.xml");
    const permissions = [...manifest.matchAll(/<uses-permission android:name="([^"]+)"/g)];
    expect(permissions.map((match) => match[1])).toEqual([
      "android.permission.INTERNET",
      "android.permission.RECORD_AUDIO",
      "android.permission.MODIFY_AUDIO_SETTINGS",
    ]);
    // Undeclared, CAMERA needs no grant for photos from <input capture> (the camera app takes them).
    expect(manifest).not.toMatch(/<uses-permission[^>]*CAMERA/);
    // Devices without a microphone can still install the app.
    expect(manifest).toContain(
      '<uses-feature android:name="android.hardware.microphone" android:required="false" />'
    );
  });

  test("the page sits between the system bars and above the keyboard", () => {
    expect(activity).toContain("enableEdgeToEdge()");
    expect(activity).toContain("setOnApplyWindowInsetsListener");
    for (const inset of ["systemBars()", "displayCutout()", "ime()"]) {
      expect(activity).toContain(`WindowInsetsCompat.Type.${inset}`);
    }
    expect(activity).toContain("view.setPadding(bars.left, bars.top, bars.right, bars.bottom)");
  });
});

describe("content security policy", () => {
  test("is the web app's policy, with API traffic allowed only over IPC", () => {
    const web = directives(webPolicy());
    const native = directives(config.app.security.csp);
    expect(native.get("connect-src")).toEqual(["ipc:", "http://ipc.localhost"]);
    web.delete("connect-src");
    native.delete("connect-src");
    expect([...native]).toEqual([...web]);
  });

  test("is not loosened elsewhere in the configuration", () => {
    expect(config.app.security.dangerousDisableAssetCspModification ?? false).toBe(false);
    expect(config.app.security.devCsp).toBeUndefined();
    expect(config.app.security.assetProtocol).toBeUndefined();
  });
});

describe("capability", () => {
  test("covers the main window with the minimal set of permissions", () => {
    expect(capability.windows).toEqual(["main"]);
    const identifiers = capability.permissions.map((entry) =>
      typeof entry === "string" ? entry : entry.identifier
    );
    expect(identifiers.sort()).toEqual([
      "core:app:allow-exit",
      "core:app:default",
      "http:default",
      "opener:allow-open-url",
    ]);
  });

  test("lets the HTTP plugin reach any server the user connects to, on any port", () => {
    // `http://*` alone would not match a URL with a port (`http://192.168.1.10:3000`).
    expect(scope("http:default")).toEqual(["http://*:*", "https://*:*"]);
  });

  test("opens only web and mail links in other apps", () => {
    expect(scope("opener:allow-open-url")).toEqual(["http://*", "https://*", "mailto:*"]);
  });

  test("ships only the HTTP and opener plugins", () => {
    const manifest = read("src-tauri/Cargo.toml");
    const dependencies = manifest.split("[dependencies]")[1]?.split(/^\[/m)[0] ?? "";
    const crates = [...dependencies.matchAll(/^([a-z0-9-]+)\s*=/gm)].map((match) => match[1]);
    expect(crates).toEqual(["tauri", "tauri-plugin-http", "tauri-plugin-opener"]);
    const plugins = [...read("src-tauri/src/lib.rs").matchAll(/\.plugin\((\w+)::init\(\)\)/g)];
    expect(plugins.map((match) => match[1])).toEqual(["tauri_plugin_http", "tauri_plugin_opener"]);
  });
});

describe("microphone", () => {
  const lib = read("src-tauri/src/lib.rs");
  const microphone = read("src-tauri/src/microphone.rs");

  test("every permission request of the web view goes to the microphone-only answer", () => {
    expect(lib).toContain(".on_permission_request(|_, kind| microphone::answer(kind))");
    // The answer itself has Rust unit tests: allow the microphone, deny every other kind.
    expect(microphone).toMatch(
      /match kind \{\s*PermissionKind::Microphone => PermissionResponse::Allow,\s*_ => PermissionResponse::Deny,\s*\}/
    );
  });

  test("Linux turns on WebKitGTK media streams in setup, before the first page commits", () => {
    expect(lib).toMatch(
      /\.setup\(\|_app\| \{\s*#\[cfg\(target_os = "linux"\)\]\s*microphone::enable\(_app\)\?;/
    );
    expect(microphone).toContain("settings.set_enable_media_stream(true);");
  });

  test("only Linux builds link WebKitGTK directly, for its settings", () => {
    const manifest = read("src-tauri/Cargo.toml");
    const linux = manifest.split(`[target.'cfg(target_os = "linux")'.dependencies]`)[1] ?? "";
    const crates = [...linux.split(/^\[/m)[0].matchAll(/^([a-z0-9-]+)\s*=/gm)].map(
      (match) => match[1]
    );
    expect(crates).toEqual(["webkit2gtk"]);
  });
});

describe("workspace", () => {
  test("the shell builds outside the root Cargo workspace and inside the pnpm workspace", () => {
    expect(read("../../Cargo.toml")).toContain('exclude = ["apps/native/src-tauri"]');
    expect(read("../../pnpm-workspace.yaml")).toMatch(/^ {2}- apps\/native$/m);
  });
});
