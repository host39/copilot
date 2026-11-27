# Copilot Native Apps

The Linux desktop and Android apps: a [Tauri](https://v2.tauri.app/) 2.12 shell around the production build of `apps/web`. The app has no server of its own. On first launch it shows **Connect to Server**, checks `GET <url>/health`, stores the address on the device, and from then on sends every API call to that server through `tauri-plugin-http`. The design is in `docs/design/copilot.md` ("Native Apps"); the research behind it is `docs/research/native-and-ci.md`.

## Layout

| Path | What it is |
| --- | --- |
| `package.json` | `@copilot/native`: the Tauri CLI (`pnpm --filter @copilot/native tauri ...`) and the configuration tests |
| `src-tauri/Cargo.toml` | The `copilot-native` crate. It is excluded from the root Cargo workspace, so `make check` never needs WebKitGTK; it has its own `Cargo.lock` and `target/` |
| `src-tauri/tauri.conf.json` | Name (`Copilot`), identifier (`com.host39.copilot`), version (from `package.json`), the web build it bundles, the window, and the Content-Security-Policy |
| `src-tauri/capabilities/default.json` | Everything the page may ask of the shell (see [Security](#security)) |
| `src-tauri/src/lib.rs` | Registers the two plugins, `http` and `opener`, and starts the app |
| `src-tauri/icons/` | Generated from `apps/web/public/icons/icon.svg` |
| `src-tauri/gen/android/` | The Android Studio project from `tauri android init`, committed because it is customized (see [Android](#android)) |
| `test/config.test.ts` | Checks the rules below: identifier, CSP, capability scopes, plugin list, Android signing and insets |

## Prerequisites

- The repository's own setup (`pnpm install` at the root, Rust stable from `rust-toolchain.toml`).
- **Linux desktop**: WebKitGTK 4.1 and friends. On Debian and Ubuntu: `sudo apt-get install libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev`. Other distributions: <https://v2.tauri.app/start/prerequisites/>.
- **Android**: JDK 21, the Android SDK with platform 37 and build tools, NDK 29 (NDK 30 is untested with Tauri 2.12), and the Rust target `rustup target add aarch64-linux-android` (add `x86_64-linux-android` for an emulator). Set `JAVA_HOME`, `ANDROID_HOME`, and `NDK_HOME` (for example `$ANDROID_HOME/ndk/29.0.13846066`).

## Run

| Command | What it does |
| --- | --- |
| `make native-dev` | Starts the web dev server on port 4000 and opens the desktop app on it, with live reload. Run the API too (`make dev-api`) and connect to `http://localhost:3000` |
| `make android-dev` | The same on a connected device or emulator. For a phone that must reach the dev server over the network, run `pnpm --filter @copilot/native tauri android dev --host`: Tauri sets `TAURI_DEV_HOST`, and the Rsbuild config listens there |
| `make native-lint` | `rustfmt --check` and Clippy with `-D warnings` for the shell (needs the Linux packages) |
| `pnpm --filter @copilot/native test` | The configuration tests (part of `make test`; no WebKitGTK needed) |

Loopback and private addresses (`localhost`, `10.x`, `172.16.x` to `172.31.x`, `192.168.x`, `100.64.x` to `100.127.x`, `fc00::/7`) connect over plain `http://` without a prompt. Any other `http://` address needs the user to confirm "Connect without encryption" first; use HTTPS through a reverse proxy for servers outside the local network. The sign-in screens show the server with a **Change** link back to the Connect screen; changing the server signs out.

## Build

| Command | Output |
| --- | --- |
| `make native-build` | `src-tauri/target/release/bundle/deb/Copilot_<version>_amd64.deb`. `pnpm --filter @copilot/native tauri build` without `--bundles` also builds the `.rpm` and the AppImage |
| `make android-build` | `src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release.apk`, for arm64 phones. Install it with `adb install -r <file>` or by opening it on the phone |

Build the `.deb` on the oldest distribution you want to support: it requires that system's glibc or newer. CI (roadmap step P5.2) builds it on Ubuntu 24.04.

The version comes from `version` in `package.json`; Android's version code is derived from it (`major * 1000000 + minor * 1000 + patch`), so bump it for every APK you hand out.

## Security

- **CSP**: `app.security.csp` is the web app's policy (`CSP` in `apps/api/src/routes/web.rs`) with `connect-src ipc: http://ipc.localhost`. The page itself may load nothing from the network; API calls leave from Rust through the HTTP plugin, which neither CORS nor mixed-content rules nor Android's cleartext policy apply to. A test keeps the two policies in step.
- **Capability** (`capabilities/default.json`), for the `main` window only:
  - `http:default` scoped to `http://*:*` and `https://*:*`. The user picks the server at run time, so any host is allowed; the explicit port wildcard is needed, because `http://*` does not match `http://192.168.1.10:3000`. The web client sends the bearer token only to the stored server's origin.
  - `opener:allow-open-url` scoped to `http://*`, `https://*`, and `mailto:*`. The opener plugin opens links with `target="_blank"` (every link in a chat) in the system browser or mail app.
  - `core:app:default` and `core:app:allow-exit`, for Android's back button (`onBackButtonPress`) and leaving the app from the chat list.
  - Nothing else: no file system, shell, window, or event permissions. `build.removeUnusedCommands` strips the commands no capability allows.
- `useHttpsScheme` stays `false`. Changing it later moves the WebView's storage (the server address and the session token) on Android and Windows.

## Android

The committed project differs from what `tauri android init` writes in two places. Re-apply both after regenerating it:

1. `app/build.gradle.kts`: the `release` build type is signed with the SDK's debug key (`signingConfig = signingConfigs.getByName("debug")`). The APK installs by sideloading without a release keystore; app stores refuse it. Each machine (and each CI runner) has its own debug key, so an APK built elsewhere cannot update an installed one in place: uninstall first, which also forgets the stored server and session. A release keystore replaces this before any store release.
2. `MainActivity.kt` pads the WebView's parent by the system bar, display cutout, and keyboard insets. Android 15 and later draw apps edge to edge, and the WebView does not offset the page by itself.

Release builds keep `usesCleartextTraffic="false"` for the WebView. That does not affect `http://` servers on the LAN, because API calls go through the plugin, not the WebView.

Regenerating:

```bash
# Icons (also writes the Android launcher icons); then delete the ones the apps do not use
pnpm --filter @copilot/native tauri icon ../web/public/icons/icon.svg
rm -rf apps/native/src-tauri/icons/ios apps/native/src-tauri/icons/Square*Logo.png apps/native/src-tauri/icons/StoreLogo.png

# Android project (overwrites the two customizations above)
pnpm --filter @copilot/native tauri android init
```

## Changing the Identifier to `in.host39.copilot`

The app's identifier is `com.host39.copilot`. Tauri also turns the identifier into the Kotlin package of the Android project and into the JNI symbol names of the Rust library, and `in` is a Kotlin keyword, so `package in.host39.copilot` does not compile and setting `"identifier": "in.host39.copilot"` breaks the Android build. To publish the app as `in.host39.copilot`:

1. Leave `identifier` in `src-tauri/tauri.conf.json` at `com.host39.copilot`. It keeps naming the Kotlin package (`app/src/main/java/com/host39/copilot/`), the Gradle `namespace`, and the JNI bindings.
2. In `src-tauri/gen/android/app/build.gradle.kts`, change only the store-facing id: `applicationId = "in.host39.copilot"`. Android identifies the installed app, its data, and its store listing by the application id; the namespace is a build-time detail. The FileProvider authority follows it (`${applicationId}.fileprovider`).
3. Optionally, for the Linux app, add `src-tauri/tauri.linux.conf.json` with `{ "identifier": "in.host39.copilot" }`. It is merged only into Linux builds and names the desktop entry and the data folder (`~/.local/share/in.host39.copilot`).
4. Update the expectations in `test/config.test.ts`, then build the APK once (CI or `make android-build`) to confirm the Tauri CLI leaves the edited `applicationId` alone.

Do it before handing out the first build, if at all: a different application id is a different app to Android (no in-place update, its own data), and on Linux a different identifier moves the stored server and session.

## Verify on the First Build

These need a real build or device; the list is section 12 of `docs/research/native-and-ci.md` plus the roadmap's P5.4 checkpoint: the Android file picker and camera capture, the back button, edge-to-edge insets, `getUserMedia` on Android, WebKitGTK's media stream setting and GStreamer packages on Linux (P5.3), and downloads of saved files (`<a download>` from a `blob:` URL) on both platforms.
