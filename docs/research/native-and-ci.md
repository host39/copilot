# Native Apps and CI Runners

Research note for wrapping `apps/web` with Tauri v2 (Linux desktop and Android first, Windows and macOS later) and for choosing GitHub-hosted runners. Checked against live primary sources on 2026-11-26. Every load-bearing claim has its source next to it. Claims marked **UNVERIFIED** could not be confirmed from a primary source or a local test; check them on a real build before depending on them.

Treat this note as a draft: versions, runner images, and prices below are a snapshot and move weekly.

## Key Findings

1. **Use Tauri 2.12.x, not 3.0.** Stable is `tauri` 2.12.7 and CLI 2.12.6 (both 2026-11-18). A `3.0.0-alpha.6` exists (2026-11-06, MSRV 1.95). `tauri-action` v1 dropped support for "Tauri v1 and unstable v2 (alpha, beta, rc) versions"; its changelog says nothing about 3.0 prereleases, so whether v1 can build a 3.0 alpha is **UNVERIFIED**. Sources: <https://crates.io/api/v1/crates/tauri>, <https://raw.githubusercontent.com/tauri-apps/tauri-action/dev/CHANGELOG.md>.
2. **Keep the Tauri crate out of the root Cargo workspace.** Building `tauri` on Linux needs WebKitGTK headers, so a workspace member would make `make check`, `cargo clippy --workspace`, and every CI Rust job depend on `libwebkit2gtk-4.1-dev`. Add `exclude = ["apps/native/src-tauri"]`; Cargo refuses to build a nested non-member package without it (tested locally, see Section 2).
3. **Our CORS layer cannot allow the Linux webview origin today.** The Linux and macOS webview origin is `tauri://localhost`; Windows and Android use `http://tauri.localhost` (or `https://tauri.localhost` with `useHttpsScheme`). `packages/middleware/src/cors.rs` drops any origin whose scheme is not `http` or `https`, so `tauri://localhost` is rejected whatever `CORS_ALLOWED_ORIGINS` says. Sources: <https://raw.githubusercontent.com/tauri-apps/tauri/tauri-v2.12.7/crates/tauri-utils/src/config.rs> (lines 2213 to 2223), <https://raw.githubusercontent.com/tauri-apps/plugins-workspace/http-v2.9.0/plugins/http/src/commands.rs> (lines 368 to 378).
4. **Route API calls through `tauri-plugin-http` in native builds.** It sends requests from Rust, so it sidesteps CORS, a possible mixed-content block on Linux (the `tauri` scheme is registered as secure in WebKitGTK, but Tauri's own docs imply `tauri://localhost` allows mixed content; **UNVERIFIED**, see Section 5), and Android's cleartext policy, and it streams response bodies. Its URL scope must use explicit port wildcards (`http://*:*`, `https://*:*`); `http://*` does not match `http://192.168.1.10:3000` (tested locally with the plugin's own pattern parser).
5. **`ubuntu-slim` is real, cheap, and small:** 1 vCPU, 5 GB RAM, 14 GB SSD, an unprivileged container, a hard **15-minute** job timeout, $0.002 per minute on private repos, and **no Docker daemon** (no `services:`, no `container:`). Use it for Biome, markdownlint, `tsc`, the web build, and automation. Rust with Postgres, the Tauri bundle, and Android need a standard runner. Sources: <https://docs.github.com/en/actions/reference/runners/github-hosted-runners>, <https://raw.githubusercontent.com/actions/runner-images/main/images/ubuntu-slim/scripts/build/install-docker-cli.sh>.
6. **`ubuntu-latest` is now Ubuntu 26.04 (glibc 2.43).** Build the Linux desktop bundle on a pinned `ubuntu-24.04` (glibc 2.39) so it still runs on Ubuntu 24.04 desktops. Sources: <https://github.com/actions/runner-images/issues/14748>, <https://raw.githubusercontent.com/actions/runner-images/main/README.md>, Launchpad `libc6` publishing records (Section 3).
7. **Microphone access needs extra work on both platforms.** Android: declare `RECORD_AUDIO` and `MODIFY_AUDIO_SETTINGS`; wry then asks the user at runtime. Linux: WebKitGTK's `enable-media-stream` defaults to `FALSE` and wry does not turn it on, so the app must enable it and answer the permission request (Tauri 2.12.0 added `on_permission_request`).

## 1. Tauri v2 Versions

Queried on 2026-11-26.

| Package | Latest stable | Published | Source |
| --- | --- | --- | --- |
| `tauri` (crate) | 2.12.7 | 2026-11-18 | <https://crates.io/api/v1/crates/tauri> |
| `tauri-build` (crate) | 2.7.6 | 2026-11-18 | <https://crates.io/api/v1/crates/tauri-build> |
| `tauri-cli` (crate) | 2.12.6 | 2026-11-18 | <https://crates.io/api/v1/crates/tauri-cli> |
| `@tauri-apps/cli` (npm) | 2.12.6 | 2026-11-18 | `npm view @tauri-apps/cli` (dist-tags: `latest` 2.12.6, `next` 3.0.0-alpha.6) |
| `@tauri-apps/api` (npm) | 2.12.5 | 2026-11-16 | `npm view @tauri-apps/api version` |
| `tauri-plugin-http` / `@tauri-apps/plugin-http` | 2.9.0 / 2.9.0 | 2026-11-10 | crates.io API, npm |
| `tauri-plugin-opener` / `@tauri-apps/plugin-opener` | 2.7.3 / 2.7.3 | 2026-11-10 | crates.io API, npm |
| `tauri-plugin-store` / `@tauri-apps/plugin-store` | 2.5.1 / 2.5.1 | 2026-11-10 | crates.io API, npm |
| `tauri-plugin-notification` / `@tauri-apps/plugin-notification` | 2.8.1 / 2.8.1 | 2026-11-10 | crates.io API, npm |
| `tauri-plugin-os` / `@tauri-apps/plugin-os` | 2.4.0 / 2.4.0 | 2026-10-14 | crates.io API, npm |
| `wry` (webview layer) | 0.58.2 is the newest on crates.io, but Tauri 2.12.7 resolves 0.57.x (see below) | 0.58.2: 2026-11-26; 0.57.4: 2026-11-25 | <https://crates.io/api/v1/crates/wry> |
| `tauri-apps/tauri-action` | v1.0.0 (commit `1deb371b0cd8bd54025b384f1cd735e725c4060f`) | n/a | `git ls-remote --tags https://github.com/tauri-apps/tauri-action` |

- `tauri` 2.12.7 depends on `tauri-runtime-wry` `~2.12.5`, which requires `wry` `^0.57.0`, so a Tauri 2.12.7 app builds against wry 0.57.4, not 0.58.x (0.58.0 shipped one day after 0.57.4 and is a breaking 0.x bump). Sources: <https://crates.io/api/v1/crates/tauri/2.12.7/dependencies>, <https://crates.io/api/v1/crates/tauri-runtime-wry/2.12.5/dependencies>. The wry files this note cites at tag `wry-v0.58.2` were re-checked at `wry-v0.57.4`: `src/android/kotlin/RustWebChromeClient.kt` is byte-identical, and `src/webkitgtk/mod.rs` has the same WebGL/WebAudio settings and permission-request wiring (0.57.4 lines 440 to 442 and 594 to 665).
- `tauri` 2.12.7 declares `rust-version = 1.90` on crates.io; our workspace pins `rust-version = "1.94"`, so no conflict. The 3.0 alphas require 1.95. Source: <https://crates.io/api/v1/crates/tauri> (`versions[].rust_version`).
- The JS packages and the crates are versioned separately (`@tauri-apps/api` 2.12.5 next to `tauri` 2.12.7 is normal). Pin each to `^2` in manifests and let the lockfiles fix exact versions.
- Use the npm `@tauri-apps/cli`: it ships prebuilt binaries as optional dependencies (`@tauri-apps/cli-linux-x64-gnu` and others) and has no install script, so pnpm 10's build-script blocking does not affect it. `cargo install tauri-cli` compiles the CLI from source on every cold CI run. Source: `npm view @tauri-apps/cli@2.12.6 optionalDependencies scripts.postinstall`.
- Tauri 3.0 alpha.0 changed the test mock origin to `tauri://localhost` on every platform and moved the Android binding macro. Re-check the origin list in Section 5 before any 3.0 upgrade. Source: <https://newreleases.io/project/github/tauri-apps/tauri/release/tauri-v3.0.0-alpha.0> (secondary; the primary is the tauri-apps/tauri GitHub release of the same name).

## 2. Project Layout in This Monorepo

### Proposed Tree

```text
apps/native/
  package.json              # "@copilot/native": devDependency @tauri-apps/cli, script "tauri": "tauri"
  src-tauri/
    Cargo.toml              # package "copilot-native", own [workspace]-less manifest, excluded from root
    Cargo.lock              # committed (Tauri docs: SHOULD commit)
    build.rs                # tauri_build::build()
    tauri.conf.json         # plus tauri.linux.conf.json / tauri.android.conf.json overrides
    capabilities/default.json
    icons/                  # `pnpm tauri icon`
    src/lib.rs              # #[cfg_attr(mobile, tauri::mobile_entry_point)] pub fn run()
    src/main.rs             # desktop entry, calls copilot_native_lib::run()
    gen/android/            # from `tauri android init`, committed (we customize manifest and Gradle)
```

- Tauri's layout and file roles (`tauri.conf.json` marks the project for the CLI, `lib.rs` holds the mobile entry point, `capabilities/` holds permissions). Source: <https://raw.githubusercontent.com/tauri-apps/tauri-docs/v2/src/content/docs/start/project-structure.mdx>.
- "You **SHOULD** commit the `src-tauri/Cargo.lock`" and should not commit `src-tauri/target`. Source: <https://raw.githubusercontent.com/tauri-apps/tauri-docs/v2/src/content/docs/develop/index.mdx>.
- The generated Android project ships its own `.gitignore` that already ignores `build`, `local.properties`, `keystore.properties`, `key.properties`, `/.tauri`, generated JNI libs, and `app/tauri.properties`, so committing `gen/android` is safe. Sources: <https://raw.githubusercontent.com/tauri-apps/tauri/tauri-cli-v2.12.6/crates/tauri-cli/templates/mobile/android/.gitignore>, `.../templates/mobile/android/app/.gitignore` (same tag).
- Add `apps/native` to `pnpm-workspace.yaml` so `pnpm --filter @copilot/native tauri ...` works and the CLI version is locked in `pnpm-lock.yaml`.

### Pointing Tauri at Rsbuild

`apps/native/src-tauri/tauri.conf.json`, build section (paths are relative to the config file):

```json
{
  "build": {
    "beforeDevCommand": "pnpm --filter @copilot/web dev",
    "devUrl": "http://localhost:4000",
    "beforeBuildCommand": "pnpm --filter @copilot/web build",
    "frontendDist": "../../web/dist"
  }
}
```

- `devUrl` plus `beforeDevCommand` is the documented way to use a bundler's dev server; `frontendDist` "relative to the configuration file" is "read recursively and all files are embedded in the application binary". Sources: <https://raw.githubusercontent.com/tauri-apps/tauri-docs/v2/src/content/docs/develop/index.mdx>, config.rs at tag `tauri-v2.12.7` (lines 3847 to 3868).
- Hook commands also accept `{ "script": ..., "cwd": ... }`; Tauri sets `TAURI_ENV_PLATFORM`, `TAURI_ENV_ARCH`, `TAURI_ENV_TARGET_TRIPLE`, and `TAURI_ENV_DEBUG` for them, so the Rsbuild build can tell it is building for a native target. Source: config.rs at tag `tauri-v2.12.7` (lines 3733 to 3762, 3869 to 3876).
- Rsbuild 2 output goes to `dist` by default (`output.distPath.root: 'dist'`). Source: <https://raw.githubusercontent.com/web-infra-dev/rsbuild/main/website/docs/en/config/output/dist-path.mdx>.
- Rsbuild 2 `server.host` defaults to `localhost` (not `0.0.0.0`). For `tauri android dev` on a device, set `server.host` and `dev.client.host` from `process.env.TAURI_DEV_HOST`, as Tauri's Vite example does. Sources: <https://raw.githubusercontent.com/web-infra-dev/rsbuild/main/website/docs/en/config/server/host.mdx>, <https://raw.githubusercontent.com/web-infra-dev/rsbuild/main/website/docs/en/config/dev/client.mdx>, <https://raw.githubusercontent.com/tauri-apps/tauri-docs/v2/src/content/docs/develop/index.mdx> (Development Server section).
- React Router's browser routing works without changes: when an asset is missing, Tauri tries `<path>.html`, then `<path>/index.html`, then falls back to `index.html`. Source: <https://raw.githubusercontent.com/tauri-apps/tauri/tauri-v2.12.7/crates/tauri/src/manager/mod.rs> (lines 384 to 413).
- `@rsbuild/core` is at 2.3.5 on npm (`npm view @rsbuild/core version`), matching `apps/web/package.json`.

### Root Cargo Workspace: Exclude the Tauri Crate

Recommendation: **exclude**, with a separate `Cargo.lock` and target directory under `apps/native/src-tauri`.

| Concern | Member of root workspace | Excluded (recommended) |
| --- | --- | --- |
| `make check` / `cargo clippy --workspace` | Needs WebKitGTK dev packages on every dev machine and CI job; compiles GTK, WebKit bindings, and Tauri on every check | Unchanged |
| CI | Every Rust job installs ~apt deps and builds Tauri | Only the native jobs pay for it |
| Shared code | Could depend on `copilot-contracts` | Not needed: the SPA already talks to the API over HTTP; the native crate is a shell |
| Lockfile | One `Cargo.lock` | Two lockfiles (Dependabot needs two entries) |
| Workspace lints | Inherited | Copy the `[lints]` you want into its `Cargo.toml` |
| `make boundaries` | Native crate appears in `cargo metadata` | Unaffected |

Local test (cargo 1.97.0): a package at `apps/native/src-tauri` under a root workspace that does not list it fails with "current package believes it's in a workspace when it's not ... add the package to the `workspace.exclude` array, or add an empty `[workspace]` table to the package's manifest". With `exclude = ["apps/native/src-tauri"]` in the root `Cargo.toml`, `cargo metadata` in that folder reports the folder itself as the workspace root. Reference: <https://doc.rust-lang.org/cargo/reference/workspaces.html>. The root `rust-toolchain.toml` still applies (rustup searches upward), and the root `.cargo/config.toml` `[env]` entries also apply when Cargo runs from that folder; they are harmless (ts-rs variables).

`tauri-action` 0.6.2 fixed workspace-root detection, and Tauri's dev watcher reads `.taurignore` from the Cargo workspace root, so either layout works with the tooling. Sources: <https://raw.githubusercontent.com/tauri-apps/tauri-action/dev/CHANGELOG.md>, <https://raw.githubusercontent.com/tauri-apps/tauri-docs/v2/src/content/docs/develop/index.mdx>.

Proposed Make targets (not part of `make check`): `native-dev` (`pnpm --filter @copilot/native tauri dev`), `native-lint` (`cargo clippy --manifest-path apps/native/src-tauri/Cargo.toml --all-targets -- -D warnings`, needs the Linux packages below), `native-build`, `android-dev`, `android-build`.

## 3. Linux Desktop

### Build Dependencies

Debian and Ubuntu: `libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev`. Fedora: `webkit2gtk4.1-devel openssl-devel curl wget file libappindicator-gtk3-devel librsvg2-devel libxdo-devel` plus `dnf group install "c-development"`. Arch: `webkit2gtk-4.1 base-devel curl wget file openssl appmenu-gtk-module libappindicator-gtk3 librsvg xdotool`. Source: <https://raw.githubusercontent.com/tauri-apps/tauri-docs/v2/src/content/docs/start/prerequisites.mdx>.

Tauri's own GitHub workflow installs `libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf xdg-utils` on Ubuntu runners. Source: <https://raw.githubusercontent.com/tauri-apps/tauri-docs/v2/src/content/docs/distribute/Pipelines/github.mdx>.

`libwebkit2gtk-4.1-dev` is published for Ubuntu 24.04 (`2.52.6-0ubuntu0.24.04.2`) and 26.04 (`2.52.6-0ubuntu0.26.04.2`). Source: Launchpad API, `getPublishedBinaries?binary_name=libwebkit2gtk-4.1-dev` for `noble/amd64` and `resolute/amd64` (<https://api.launchpad.net/1.0/ubuntu/+archive/primary>).

### glibc Baseline

- Tauri: "you must build your Tauri application using the oldest base system you intend to support that also provides Tauri v2's required WebKitGTK 4.1 packages. Ubuntu 22.04 and Debian 12 are suitable baseline examples." Source: <https://raw.githubusercontent.com/tauri-apps/tauri-docs/v2/src/content/docs/distribute/appimage.mdx> (same text in `debian.mdx` and `rpm.mdx`).
- glibc is 2.39 on Ubuntu 24.04 and 2.43 on Ubuntu 26.04 (Launchpad `libc6` published versions `2.39-0ubuntu8.12` and `2.43-2ubuntu2.7`).
- `ubuntu-latest` moved to Ubuntu 26.04, rolled out from 2026-10-19 with completion targeted for 2026-11-19. Sources: <https://github.com/actions/runner-images/issues/14748>, <https://raw.githubusercontent.com/actions/runner-images/main/README.md> (table lists `ubuntu-latest` as Ubuntu 26.04).
- Ubuntu 22.04 runner images began deprecation on September 17 and are "fully unsupported by April 17". Source: announcement banner in <https://raw.githubusercontent.com/actions/runner-images/main/images/ubuntu-slim/ubuntu-slim-Readme.md>.
- Decision: build Linux bundles on **`ubuntu-24.04`**. That supports Ubuntu 24.04+, Debian 13, and current Fedora. To reach Debian 12 (glibc 2.36) later, build inside a `debian:bookworm` job container on a standard runner (**UNVERIFIED** in our setup).

### Bundle Targets and Sizes

- Linux formats: Debian package, RPM, AppImage (plus Snap, Flatpak, AUR through other channels). `tauri build` bundles the formats in `bundle.targets`; `--bundles deb,rpm,appimage` limits them. Source: <https://raw.githubusercontent.com/tauri-apps/tauri-docs/v2/src/content/docs/distribute/index.mdx>.
- The `.deb` declares `libwebkit2gtk-4.1-0` and `libgtk-3-0` (and `libappindicator3-1` with a tray icon). Source: <https://raw.githubusercontent.com/tauri-apps/tauri-docs/v2/src/content/docs/distribute/debian.mdx>.
- Size: an AppImage bundles its dependencies, so "the file size grows from the 2-6 MB range to 70+ MB". A minimal Tauri app "can be less than 600KB". Sources: <https://raw.githubusercontent.com/tauri-apps/tauri-docs/v2/src/content/docs/distribute/appimage.mdx>, <https://raw.githubusercontent.com/tauri-apps/tauri-docs/v2/src/content/docs/start/index.mdx>. Our own sizes are **UNVERIFIED** until the first build; expect a few MB for `.deb`/`.rpm`.
- Audio: an AppImage that plays audio or video needs `bundle > linux > appimage > bundleMediaFramework: true`, which adds GStreamer files and is "only fully supported on Ubuntu build systems". Source: <https://raw.githubusercontent.com/tauri-apps/tauri-docs/v2/src/content/docs/distribute/appimage.mdx>. Relevant only if voice playback ships (see `docs/research/voice.md`).
- Size settings: `[profile.release] codegen-units = 1, lto = true, opt-level = "s", panic = "abort", strip = true`, and `build.removeUnusedCommands: true` (requires `tauri@2.4`+). Source: <https://raw.githubusercontent.com/tauri-apps/tauri-docs/v2/src/content/docs/concept/size.mdx>. These live in the excluded crate's own manifest, so they do not change the API's release profile.
- ARM: `linuxdeploy` cannot cross-compile ARM AppImages; native `ubuntu-24.04-arm` runners work and "a typical build should take ~10 minutes", while QEMU emulation needs about an hour uncached. Sources: `appimage.mdx` and `Pipelines/github.mdx` (links above).

## 4. Android

### Toolchain Versions (Tauri CLI 2.12.6 Templates)

| Item | Value | Source |
| --- | --- | --- |
| `compileSdk` / `targetSdk` | 37 / 37 | <https://raw.githubusercontent.com/tauri-apps/tauri/tauri-cli-v2.12.6/crates/tauri-cli/templates/mobile/android/app/build.gradle.kts> |
| Default `minSdk` | 24 (Android 7.0); override with `bundle > android > minSdkVersion` | config.rs at `tauri-v2.12.7` (`default_min_sdk_version`), <https://raw.githubusercontent.com/tauri-apps/tauri-docs/v2/src/content/docs/distribute/google-play.mdx> |
| Android Gradle Plugin / Kotlin plugin | 9.3.1 / 2.2.10 | `templates/mobile/android/build.gradle.kts` (same tag) |
| Gradle wrapper | 9.6.1 | `templates/mobile/android/gradle/wrapper/gradle-wrapper.properties` (same tag) |
| NDK the CLI installs when missing | 29.0.13846066 (with `platforms;android-37`) | <https://raw.githubusercontent.com/tauri-apps/tauri/tauri-cli-v2.12.6/crates/tauri-cli/src/mobile/android/mod.rs> (lines 53 to 54, 683 to 694) |
| Rust targets | `aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android` | <https://raw.githubusercontent.com/tauri-apps/tauri-docs/v2/src/content/docs/start/prerequisites.mdx> |
| Env vars | `JAVA_HOME`, `ANDROID_HOME`, `NDK_HOME` | same prerequisites page |

The CLI reads `NDK_HOME`, else picks the newest folder under `$ANDROID_HOME/ndk`. Source: `mobile/android/mod.rs` (lines 709 to 739, same tag).

GitHub's Ubuntu 24.04 and 26.04 images ship Android SDK platforms up to `android-37.2`, build-tools 37.0.0, NDK `27.3.13750724` (default), `29.0.14206865`, and `30.0.16248370`, with `ANDROID_HOME`, `ANDROID_NDK_HOME`, and `ANDROID_NDK_LATEST_HOME` set. Java: 17 (default) on 24.04, 25 (default) on 26.04, 21 on both. Sources: <https://raw.githubusercontent.com/actions/runner-images/main/images/ubuntu/Ubuntu2404-Readme.md>, <https://raw.githubusercontent.com/actions/runner-images/main/images/ubuntu/Ubuntu2604-Readme.md>. Whether Tauri 2.12 builds with NDK 30 is **UNVERIFIED**; point `NDK_HOME` at the 29.x folder.

### Commands

- `tauri android init` generates `src-tauri/gen/android`. `tauri android dev` runs on a device or emulator; `--open` opens Android Studio. Source: <https://raw.githubusercontent.com/tauri-apps/tauri-docs/v2/src/content/docs/develop/index.mdx>.
- `tauri android build --apk` builds APKs, `--aab` builds App Bundles, `--target aarch64 --target armv7` limits ABIs (default is all four, universal), `--split-per-abi` emits one file per ABI. Source: <https://raw.githubusercontent.com/tauri-apps/tauri-docs/v2/src/content/docs/distribute/google-play.mdx>.
- Other flags: `--debug`, `--features`, `--config` (and per-platform `tauri.android.conf.json` merged automatically), `--ci` (also set from the `CI` env var, which GitHub sets). Source: <https://raw.githubusercontent.com/tauri-apps/tauri/tauri-cli-v2.12.6/crates/tauri-cli/src/mobile/android/build.rs> (lines 36 to 80).
- Version code defaults to `major*1000000 + minor*1000 + patch`, or `bundle > android > versionCode`. Source: `google-play.mdx`.
- For sideloading, build `--apk --target aarch64` (nearly all current phones). Add `x86_64` only for emulators. APK size is **UNVERIFIED** until the first build.

### Signing: Debug vs Release

- Debug builds are signed with the SDK's debug keystore (`$HOME/.android/debug.keystore`, created automatically). Stores, including Google Play, reject debug-signed apps. Source: <https://developer.android.com/studio/publish/app-signing>. On CI the debug keystore is new on every runner, so debug APKs from different runs cannot upgrade each other in place.
- Release signing: generate a keystore with `keytool -genkey -v -keystore ~/upload-keystore.jks -keyalg RSA -keysize 2048 -validity 10000 -alias upload`, write `gen/android/keystore.properties` (`password`, `keyAlias`, `storeFile`), and add a `signingConfigs { create("release") { ... } }` block plus `signingConfig = signingConfigs.getByName("release")` in `gen/android/app/build.gradle.kts`. In CI, store `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD`, and the base64 keystore `ANDROID_KEY_BASE64` as secrets and write the file in a step. Source: <https://raw.githubusercontent.com/tauri-apps/tauri-docs/v2/src/content/docs/distribute/Sign/android.mdx>.
- Google Play requires a key valid past 2033-10-22 and recommends 25+ years. Source: <https://developer.android.com/studio/publish/app-signing>.
- Without a `signingConfig`, the release APK is unsigned and will not install (**UNVERIFIED** for Tauri's output naming; standard Android Gradle Plugin behavior). Plan: PR builds compile a release APK without uploading it; `main` and tag builds sign it.

### Sideloading Policy Change

Google's developer verification: from 2026-09-30, apps "must be registered by verified developers in order to be installed and updated on certified Android devices" in Brazil, Indonesia, Singapore, and Thailand, rolling out globally in 2027. Free limited-distribution accounts (students and hobbyists, up to 20 devices, no government ID) and an "advanced flow" for power users launched in August 2026; ADB installs stay allowed. The post carries an update saying details changed; check <https://developer.android.com/developer-verification> before the first public APK. Source: <https://android-developers.googleblog.com/2026/03/android-developer-verification-rolling-out-to-all-developers.html>.

### Network Access to a User-Configured Server

- The template manifest already requests `android.permission.INTERNET`. Source: <https://raw.githubusercontent.com/tauri-apps/tauri/tauri-cli-v2.12.6/crates/tauri-cli/templates/mobile/android/app/src/main/AndroidManifest.xml>.
- The template sets `usesCleartextTraffic` to `"true"` for debug and `"false"` for release through `manifestPlaceholders`. Source: `app/build.gradle.kts` (same tag). So **release builds cannot reach `http://` LAN servers from the WebView by default.**
- Android: cleartext is disabled by default from API 28; `"WebView honors this attribute for applications targeting API level 26 and higher"`; the attribute "is ignored on Android 7.0 (API level 24) and above if an Android Network Security Config is present", and "will be ignored for apps targeting API levels 38 and above". Source: <https://developer.android.com/guide/topics/manifest/application-element>.
- Network Security Config has no IP-range or wildcard host syntax (`<domain includeSubdomains>` takes a host name); app-wide cleartext uses `<base-config cleartextTrafficPermitted="true">`, which Google says "should be avoided whenever possible". From API 37, `localhost` and loopback IPs get an implicit cleartext-allowed config. Source: <https://developer.android.com/privacy-and-security/security-config>.
- Options for a self-hosted app where users type LAN IPs:
  1. **Recommended:** send API traffic through `tauri-plugin-http` (Section 5). Requests leave from Rust sockets, which Android does not expect to honor the cleartext flag ("there's no expectation that the Socket API honors this flag"; source: the `application-element` page above). Keep the WebView itself cleartext-free.
  2. Add `res/xml/network_security_config.xml` with `<base-config cleartextTrafficPermitted="true">` and reference it from the manifest. Simple, but app-wide and flagged as insecure by Google.
  3. Require HTTPS on the server (reverse proxy). Best security, worst first-run experience on a LAN.
- Keep `useHttpsScheme` at its default `false`: with `https://tauri.localhost` the WebView "will NOT allow mixed content when trying to fetch `http` endpoints", and changing the value later moves IndexedDB, cookies, and localStorage. Source: config.rs at tag `tauri-v2.12.7` (lines 2213 to 2223).

### Microphone (`RECORD_AUDIO`) and `getUserMedia`

- wry's Android `RustWebChromeClient.onPermissionRequest` maps `RESOURCE_AUDIO_CAPTURE` to `Manifest.permission.MODIFY_AUDIO_SETTINGS` and `RECORD_AUDIO`, calls `activity.requestPermissions(...)` on Android 6+, and grants the WebView request only if the user agrees. With no permission handler registered, the native callback returns "default" and wry keeps audio capture in its list of known resources, so the runtime prompt still fires. Source: <https://raw.githubusercontent.com/tauri-apps/wry/wry-v0.58.2/src/android/kotlin/RustWebChromeClient.kt> (lines 60 to 118; the file is identical at `wry-v0.57.4`, the version Tauri 2.12.7 resolves), <https://raw.githubusercontent.com/tauri-apps/wry/wry-v0.57.4/src/android/binding.rs> (`onPermissionRequestNative`, lines 497 to 512).
- Runtime requests only work for permissions declared in the manifest, so add to `gen/android/app/src/main/AndroidManifest.xml`: `android.permission.RECORD_AUDIO`, `android.permission.MODIFY_AUDIO_SETTINGS`, and `<uses-feature android:name="android.hardware.microphone" android:required="false" />`. A community guide for Tauri 2.10.3 reports `NotReadableError` when `MODIFY_AUDIO_SETTINGS` is missing. Source: <https://github.com/orgs/tauri-apps/discussions/12732> (community, not official).
- Tauri 2.12.0 exposed wry's permission API: `WebviewBuilder::on_permission_request` and `Builder::on_permission_request`, with Android "supported via JNI bridge for geolocation, microphone, camera, protected media, and MIDI requests". Sources: <https://raw.githubusercontent.com/tauri-apps/tauri/tauri-v2.12.7/crates/tauri/CHANGELOG.md> (2.12.0, PR #14865), <https://raw.githubusercontent.com/tauri-apps/tauri/tauri-v2.12.7/crates/tauri/src/webview/mod.rs> (lines 703 to 755).
- `getUserMedia` needs a secure context. Chromium treats `http://*.localhost` as potentially trustworthy, so `http://tauri.localhost` should qualify: **UNVERIFIED** on a device; test it before shipping the mic button.

## 5. Runtime Server URL, Origins, and CORS

### Server URL at Runtime

- Detect native builds with `isTauri()` from `@tauri-apps/api/core` (since 2.0.0; reads `globalThis.isTauri`). Source: <https://raw.githubusercontent.com/tauri-apps/tauri/tauri-v2.12.7/packages/api/src/core.ts> (lines 600 to 608).
- `apps/web/src/config.ts` today fixes `API_URL` at build time (`import.meta.env.PUBLIC_API_URL ?? "http://localhost:3000"`). Native builds need a runtime value: on first launch show a "Connect to server" screen, check `GET <url>/health`, store the URL, and build every request from it. Web builds keep the build-time value.
- Storage: `localStorage` is enough for the server URL and the bearer token (auth uses `Authorization: Bearer`, per `docs/design/copilot.md`, so no cross-site cookies are involved). It survives restarts and lives in the app's data directory. Do not flip `useHttpsScheme` after release (it moves localStorage; see Section 4).

### Webview Origins

| Platform | Origin of the bundled SPA | Source |
| --- | --- | --- |
| Linux, macOS (and iOS) | `tauri://localhost` | plugin-http sends `"tauri://localhost"` for the `tauri` scheme because the `url` crate serializes it as `null`: <https://raw.githubusercontent.com/tauri-apps/plugins-workspace/http-v2.9.0/plugins/http/src/commands.rs> (lines 368 to 378) |
| Windows, Android (default) | `http://tauri.localhost` | `useHttpsScheme` docs: custom protocols use "`https://<scheme>.localhost` instead of the default `http://<scheme>.localhost` on Windows and Android": config.rs at `tauri-v2.12.7` (lines 2213 to 2223); Windows moved from `https` to `http` in v2: <https://raw.githubusercontent.com/tauri-apps/tauri-docs/v2/src/content/docs/start/migrate/from-tauri-1.mdx> |
| Windows, Android with `useHttpsScheme: true` | `https://tauri.localhost` | same config.rs lines |
| `tauri dev` | the dev server, `http://localhost:4000` | `devUrl` |

### What Our API Must Change

- `packages/middleware/src/cors.rs` `hostname()` returns `None` unless the scheme is `http` or `https`, and `CORS_ALLOWED_ORIGINS` matches host names only. Result: `tauri://localhost` can never pass, and `http://tauri.localhost` passes only if an operator adds `tauri.localhost`.
- If the webview's own `fetch` is used anywhere, add an explicit native-origins rule (exact matches on `tauri://localhost`, `http://tauri.localhost`, `https://tauri.localhost`), on by default or behind one env flag, and keep the hostname rules for web origins. Bearer-token auth means CORS is not what protects user data here.
- With `tauri-plugin-http` (below), the request is not subject to CORS ("The request is performed by the Rust backend instead of the webview, so it is not subject to CORS", "but the URL must be allowed by the plugin scope"; source: <https://v2.tauri.app/reference/javascript/http/>). The plugin still sends an `Origin` header (table above); `tower-http`'s `CorsLayer` only decorates responses, so nothing breaks, but log lines will show these origins.

### Mixed Content on Linux and macOS

wry registers each custom scheme as secure on WebKitGTK (`register_uri_scheme_as_secure(name)`, "Enable secure context"). Source: <https://raw.githubusercontent.com/tauri-apps/wry/wry-v0.58.2/src/webkitgtk/web_context.rs> (lines 130 to 144). A page on a secure origin that fetches `http://192.168.x.x` is mixed content, and WebKit does not exempt loopback the way the spec allows (WebKit bug <https://bugs.webkit.org/show_bug.cgi?id=171934>). So a webview `fetch` from `tauri://localhost` to a plain-HTTP LAN server may be blocked on Linux and macOS. Counterpoint: Tauri's own `useHttpsScheme` doc says the `https` scheme "will NOT allow mixed content when trying to fetch `http` endpoints and therefore will not match the behavior of the `<scheme>://localhost` protocols used on macOS and Linux", which implies `tauri://localhost` does allow such fetches (config.rs at tag `tauri-v2.12.7`, lines 2213 to 2223). The sources disagree: **UNVERIFIED**, needs a one-line test on the first Linux build. The recommended transport below avoids the question either way.

### Recommended Transport

Make the API client's `fetch` injectable. In native builds use `fetch` from `@tauri-apps/plugin-http` (dynamic import, so the web bundle does not grow); in web builds use `window.fetch`.

- The plugin's `fetch` returns a `Response` whose body is a `ReadableStream` that pulls chunks from Rust (`plugin:http|fetch_read_body`), so SSE-over-fetch streaming of chat tokens works. Source: <https://raw.githubusercontent.com/tauri-apps/plugins-workspace/http-v2.9.0/plugins/http/guest-js/index.ts> (lines 270 to 320).
- Scope: the user picks the server at runtime, so the capability must allow any host and port:

```json
{
  "identifier": "default",
  "windows": ["main"],
  "permissions": [
    "core:default",
    "opener:default",
    {
      "identifier": "http:default",
      "allow": [{ "url": "http://*:*" }, { "url": "https://*:*" }]
    }
  ]
}
```

- Port wildcard: I ran the plugin's `parse_url_pattern` logic (copied from <https://raw.githubusercontent.com/tauri-apps/plugins-workspace/http-v2.9.0/plugins/http/src/scope.rs>, lines 17 to 34) against `urlpattern` 0.6 (the plugin's dependency). Results: `http://*` and `*://*` do **not** match `http://192.168.1.10:3000/chats`; `http://*:*` and `http://*:*/*` do; `https://*:*` matches `https://example.com:8443/a`.
- Forbidden request headers are dropped unless the `unsafe-headers` feature is on; `Authorization` and `Content-Type` are not forbidden headers, so no feature flag is needed. Source: <https://raw.githubusercontent.com/tauri-apps/tauri-docs/v2/src/content/docs/plugin/http-client.mdx>.
- `apps/web` would gain `@tauri-apps/api` and `@tauri-apps/plugin-http` as dependencies. AGENTS.md says `apps/web` "only imports `@copilot/contracts`"; that rule targets workspace packages, but record the exception in an ADR.
- With all API traffic on IPC, a strict CSP becomes possible (`connect-src ipc: http://ipc.localhost`, as in Tauri's example); CSP is off unless set in config. Source: <https://raw.githubusercontent.com/tauri-apps/tauri-docs/v2/src/content/docs/security/csp.mdx>. Audio playback should use `blob:` URLs created from fetched bytes, not `<audio src="http://...">`.

## 6. Microphone on Linux

- WebKitGTK `enable-media-stream`: "Enable or disable support for MediaStream on pages", default **FALSE**. Source: <https://webkitgtk.org/reference/webkit2gtk/stable/property.Settings.enable-media-stream.html>.
- wry 0.58.2 enables WebGL and WebAudio but not media stream in its WebKitGTK settings, and only connects `permission-request` when a permission handler is set. Source: <https://raw.githubusercontent.com/tauri-apps/wry/wry-v0.58.2/src/webkitgtk/mod.rs> (lines 443 to 470, 598 to 660).
- So on Linux the app must (a) call `webview.with_webview(...)` and set `enable-media-stream` on the inner `webkit2gtk::WebView` settings, and (b) register `on_permission_request` that returns `Allow` for `PermissionKind::Microphone` only (or `Default` to let WebKitGTK prompt). The exact `with_webview` code path is **UNVERIFIED**; a community example exists at <https://github.com/tauri-apps/tauri/discussions/8426>.
- **Update (P5.3, checked locally on 2026-11-27 with WebKitGTK 2.52.6, Ubuntu 24.04's current package):** `enable-media-stream` defaults to **TRUE** there (the installed GIR says `default-value="TRUE"`, and a fresh web view reports it on), so the FALSE above holds only for older releases. A setting changed in Tauri's `setup` reaches the first page (turning it off there removed `navigator.mediaDevices`), while a change 1.5 seconds later does not reach the loaded page. `PermissionResponse::Default` on Linux denies (`getUserMedia({ audio })` failed with `NotAllowedError` without a handler), so `Default` does not prompt; the shell answers `Allow` for the microphone and `Deny` for the rest. With WebKitGTK's mock capture devices, the microphone-only handler granted audio and denied video and audio plus video. Details: `docs/design/tracks/p5-3.md`.
- WebKitGTK captures audio through GStreamer, so the user's system needs GStreamer and PipeWire or PulseAudio source plugins: **UNVERIFIED** which packages the `.deb` must depend on.

## 7. Plugins: Keep the List Short

| Plugin | Use it? | Why |
| --- | --- | --- |
| `http` | **Yes** | API transport for native builds (Section 5): no CORS, no mixed-content block, works with LAN HTTP servers on Android release builds, streams bodies. Platforms: Windows, Linux, macOS, Android, iOS. Requires Rust 1.90+. Source: <https://raw.githubusercontent.com/tauri-apps/plugins-workspace/http-v2.9.0/plugins/http/README.md> |
| `opener` | **Yes** | Open links from chat messages in the system browser. `on_new_window` (the hook for `window.open` and `target="_blank"`) is "Not supported" on Android and iOS, so the app needs an explicit "open externally" path. Source: <https://raw.githubusercontent.com/tauri-apps/tauri/tauri-v2.12.7/crates/tauri/src/webview/mod.rs> (lines 540 to 598) |
| `store` | No (for now) | Server URL and token fit in `localStorage`. The store plugin writes a JSON file usable from Rust; adopt it only when Rust code needs the values. Source: <https://raw.githubusercontent.com/tauri-apps/tauri-docs/v2/src/content/docs/plugin/store.mdx> |
| `notification` | Later | Local notifications ("schedule run finished") while the app is open. Not needed for v1; push from the server is a separate, larger feature. Source: <https://raw.githubusercontent.com/tauri-apps/tauri-docs/v2/src/content/docs/plugin/notification.mdx> |
| `os` | No | `platform()` is a compile-time string. `isTauri()` plus `navigator.userAgent` cover our needs. Source: <https://raw.githubusercontent.com/tauri-apps/tauri-docs/v2/src/content/docs/plugin/os-info.mdx> |

## 8. GitHub-Hosted Runner Catalog (November 2026)

Specs are from <https://docs.github.com/en/actions/reference/runners/github-hosted-runners>; per-minute rates (private repos) from <https://docs.github.com/en/billing/reference/actions-runner-pricing>. Public repositories use standard runners "free and unlimited".

| Label | Kind | Public: vCPU / RAM / SSD | Private: vCPU / RAM / SSD | Arch | Private rate (USD/min) |
| --- | --- | --- | --- | --- | --- |
| `ubuntu-slim` | Container on a shared VM | 1 / 5 GB / 14 GB | same | x64 | 0.002 |
| `ubuntu-latest` (= `ubuntu-26.04`), `ubuntu-24.04`, `ubuntu-22.04` | VM | 4 / 16 GB / 14 GB | 2 / 8 GB / 14 GB | x64 | 0.006 |
| `ubuntu-26.04-arm`, `ubuntu-24.04-arm`, `ubuntu-22.04-arm` | VM | 4 / 16 GB / 14 GB | 2 / 8 GB / 14 GB | arm64 | 0.005 |
| `windows-latest`, `windows-2025`, `windows-2025-vs2026`, `windows-2022` | VM | 4 / 16 GB / 14 GB | 2 / 8 GB / 14 GB | x64 | 0.010 |
| `windows-11-arm`, `windows-11-vs2026-arm` | VM | 4 / 16 GB / 14 GB | 2 / 8 GB / 14 GB | arm64 | 0.010 |
| `macos-latest` (= macOS 26), `macos-26`, `macos-15`, `xcode-27` | VM | 3 (M1) / 7 GB / 14 GB | same | arm64 | 0.062 |
| `macos-26-intel`, `macos-15-intel` | VM | 4 / 14 GB / 14 GB | same | x64 | 0.062 |

- The Tauri docs still say Linux arm runners are "Only available in public repos"; GitHub's runner table now lists the arm64 labels for private repositories too (2 vCPU / 8 GB). Trust GitHub's page.
- macOS 14 images retired from GitHub Actions on 2026-11-02. Source: <https://raw.githubusercontent.com/actions/runner-images/main/README.md>.
- Limits: job execution time 6 hours on all GitHub-hosted runners; cache storage 10 GB per repository; concurrent jobs on the Free plan 20 (5 macOS). Source: <https://docs.github.com/en/actions/reference/limits>. Partial minutes round up to whole minutes. Source: pricing page above.

### `ubuntu-slim` in Detail

- "1 vCPU and 5 GBs RAM", runs "inside of a container rather than a dedicated VM instance", "hypervisor level 2 isolation", GA on 2026-01-22. Source: <https://github.blog/changelog/2026-01-22-1-vcpu-linux-runner-now-generally-available-in-github-actions/>.
- "The job timeout for single-CPU runners is 15 minutes. If a job reaches this limit, the job is terminated and fails." Source: <https://docs.github.com/en/actions/reference/runners/github-hosted-runners>.
- "runs in unprivileged mode ... mounting file systems, using Docker-in-Docker, or accessing low-level kernel features are not supported"; "optimized for automation tasks, issue operations and short-running jobs. They are not suitable for typical heavyweight CI/CD builds." Source: same page.
- The image installs the Docker CLI, Compose, and Buildx "but not the engine ... It cannot build or run containers locally." So `services:` (our Postgres), `container:` jobs, and Docker container actions cannot run on it. Source: <https://raw.githubusercontent.com/actions/runner-images/main/images/ubuntu-slim/scripts/build/install-docker-cli.sh>. GitHub's docs separately require "an Ubuntu runner" for service containers without mentioning slim (<https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax>).
- Software: Ubuntu 24.04.5, Node.js 24.22.3, npm, Python 3.12, git, `gh`, `jq`, `make`, `gcc`; **no Rust, no Java, no Android SDK, no PostgreSQL**. Source: <https://raw.githubusercontent.com/actions/runner-images/main/images/ubuntu-slim/ubuntu-slim-Readme.md> (image 20261116.56.1).
- GitHub's own list of good uses: "Auto-labeling issues", "Basic language compilation (e.g., webpack builds)", "Linting and formatting", "Making API calls to external tools", "Running simple python scripts". Source: GA changelog above.

### Useful Preinstalled Software on Standard Ubuntu Images

Rust 1.100.0 and Cargo, Docker server, Node.js (22 on 24.04, 24 on 26.04), Java 17/21/25, Android SDK and NDKs, and PostgreSQL (16.16 on 24.04, 18.7 on 26.04; service disabled, start with `sudo systemctl start postgresql.service`). Sources: Ubuntu 24.04 and 26.04 readmes linked in Section 4. Starting the preinstalled PostgreSQL 18 on `ubuntu-latest` could replace the `postgres:18.1` service container (saves the image pull; **UNVERIFIED** that our migrations need nothing else).

## 9. Recommended Runner per Job

| Job | Runner | Why not smaller | Notes |
| --- | --- | --- | --- |
| Biome, markdownlint, `tsc` (TypeScript 7), Rust import-path grep, web build (`rsbuild build`) | **`ubuntu-slim`** | n/a | Node 24 is preinstalled; add `pnpm/action-setup`. Set `timeout-minutes: 10`. Matches GitHub's listed uses (linting, webpack-style builds). |
| Path filter / "what changed" gate, release notes, labels | **`ubuntu-slim`** | n/a | Lets heavy native jobs skip when `apps/native/**` and `apps/web/**` did not change. |
| Rust: `cargo fmt --check`, Clippy, unit tests, DB tests with Postgres, contracts drift, `make boundaries`, Bruno smoke | **`ubuntu-latest`** (x64) | Slim has no Docker daemon (no Postgres service), no Rust, 1 vCPU, and a 15-minute cap a cold axum/sqlx/utoipa build would likely exceed (**UNVERIFIED** estimate) | For private repos, `ubuntu-24.04-arm` is ~17% cheaper per minute with the same size; `postgres` images are multi-arch, but check every third-party action on arm64 first. |
| API release build (`apps/api/Dockerfile`) | **`ubuntu-latest`** for amd64; **`ubuntu-24.04-arm`** for an arm64 image | Needs a Docker engine | Build each architecture natively instead of QEMU. For a bare binary shipped outside Docker, pin `ubuntu-24.04` (glibc 2.39 floor). |
| Linux Tauri bundle (`.deb`, `.rpm`, AppImage) | **`ubuntu-24.04`** (pinned) | Slim: 15-minute cap, 1 vCPU, cold Tauri build; `ubuntu-latest` (26.04) would raise the glibc floor to 2.43 | Add `ubuntu-24.04-arm` later for arm64 Linux. |
| Android APK / AAB | **`ubuntu-latest`** | Slim has no Java or Android SDK and the 15-minute cap | SDK, NDK 29, and Java 21 are preinstalled; glibc does not matter for Android. Build `--target aarch64` only to keep the job short. |
| macOS / iOS bundles (later) | **`macos-latest`** (arm64) | Apple tooling only exists on macOS | About 10x the Linux per-minute price on private repos; run on tags only. |
| Windows bundle (later) | **`windows-latest`** | MSVC/WebView2 toolchain | Tags only. |

Splitting today's single `quality` job (one `ubuntu-latest` runner for everything) into a slim lint job and a standard Rust job lets cheap checks fail fast and frees the bigger runner for Rust work.

## 10. Workflow Sketches

Illustrative and not run. Action pins reuse the SHAs in `.github/workflows/code-quality.yml`; the new ones were resolved with `git ls-remote`: `tauri-apps/tauri-action` v1.0.0 = `1deb371b0cd8bd54025b384f1cd735e725c4060f`, `actions/setup-java` v6.3.0 = `d1374f8ac74474200d16ef8ecee22c7ae7a51880`, `actions/upload-artifact` v7.0.2 = `cf430e030ddbb5b0abf93d22962f4752f3646cd9`.

### Linux Desktop

```yaml
native-linux:
  runs-on: ubuntu-24.04
  timeout-minutes: 45
  permissions:
    contents: read
  steps:
    - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
      with:
        persist-credentials: false
    - name: Install WebKitGTK and bundler packages
      run: |
        sudo apt-get update
        sudo apt-get install -y libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf xdg-utils
    - uses: dtolnay/rust-toolchain@39008827e6b4319a349e9a42dfc90579dd25f1ca # stable
      with:
        toolchain: stable
    - uses: Swatinem/rust-cache@d1527c8f67ef9af357290475d4af4b0fab8f12e0 # v2.9.3
      with:
        workspaces: apps/native/src-tauri -> target
        save-if: ${{ github.ref == 'refs/heads/main' }}
    - uses: pnpm/action-setup@73e46992e05b19abd31f2f3c20e0e7370d762115 # v6.1.1
    - uses: actions/setup-node@949feb2413d6458794dcd2491c4babbbce0c15c1 # v7.1.0
      with:
        node-version-file: .nvmrc
        cache: pnpm
    - run: pnpm install --frozen-lockfile
    - uses: tauri-apps/tauri-action@1deb371b0cd8bd54025b384f1cd735e725c4060f # v1.0.0
      with:
        projectPath: apps/native
        args: --bundles deb,rpm,appimage
        uploadWorkflowArtifacts: true
```

- `tauri-action` builds without creating a release when `tagName`, `releaseName`, and `releaseId` are omitted; add them (and `contents: write`) on the release workflow only. `uploadWorkflowArtifacts` uploads each bundle as a workflow artifact. `projectPath` points at the folder whose `package.json` holds `@tauri-apps/cli`; without it the action installs a global CLI. Source: <https://raw.githubusercontent.com/tauri-apps/tauri-action/dev/README.md>.
- `save-if` and `workspaces` are `rust-cache` inputs; saving only from `main` keeps the 10 GB cache budget for the default branch. Source: <https://raw.githubusercontent.com/Swatinem/rust-cache/v2.9.3/README.md>.
- PR builds can pass `--bundles deb` only: AppImage packaging is the slow step (**UNVERIFIED** timing).

### Android

```yaml
native-android:
  runs-on: ubuntu-latest
  timeout-minutes: 60
  permissions:
    contents: read
  steps:
    - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
      with:
        persist-credentials: false
    - uses: actions/setup-java@d1374f8ac74474200d16ef8ecee22c7ae7a51880 # v6.3.0
      with:
        distribution: temurin
        java-version: "21"
        cache: gradle
        cache-dependency-path: apps/native/src-tauri/gen/android/**/*.gradle*
    - name: Use the preinstalled NDK 29
      run: echo "NDK_HOME=$ANDROID_HOME/ndk/$(ls -1 "$ANDROID_HOME/ndk" | grep '^29\.' | tail -n 1)" >> "$GITHUB_ENV"
    - uses: dtolnay/rust-toolchain@39008827e6b4319a349e9a42dfc90579dd25f1ca # stable
      with:
        toolchain: stable
        targets: aarch64-linux-android
    - uses: Swatinem/rust-cache@d1527c8f67ef9af357290475d4af4b0fab8f12e0 # v2.9.3
      with:
        workspaces: apps/native/src-tauri -> target
        save-if: ${{ github.ref == 'refs/heads/main' }}
    - uses: pnpm/action-setup@73e46992e05b19abd31f2f3c20e0e7370d762115 # v6.1.1
    - uses: actions/setup-node@949feb2413d6458794dcd2491c4babbbce0c15c1 # v7.1.0
      with:
        node-version-file: .nvmrc
        cache: pnpm
    - run: pnpm install --frozen-lockfile
    - name: Write release signing config
      if: github.ref == 'refs/heads/main' || startsWith(github.ref, 'refs/tags/')
      working-directory: apps/native/src-tauri/gen/android
      env:
        KEY_ALIAS: ${{ secrets.ANDROID_KEY_ALIAS }}
        KEY_PASSWORD: ${{ secrets.ANDROID_KEY_PASSWORD }}
        KEY_BASE64: ${{ secrets.ANDROID_KEY_BASE64 }}
      run: |
        printf '%s' "$KEY_BASE64" | base64 -d > "$RUNNER_TEMP/keystore.jks"
        {
          echo "keyAlias=$KEY_ALIAS"
          echo "password=$KEY_PASSWORD"
          echo "storeFile=$RUNNER_TEMP/keystore.jks"
        } > keystore.properties
    - run: pnpm --filter @copilot/native tauri android build --apk --target aarch64
    - uses: actions/upload-artifact@cf430e030ddbb5b0abf93d22962f4752f3646cd9 # v7.0.2
      if: github.ref == 'refs/heads/main' || startsWith(github.ref, 'refs/tags/')
      with:
        name: android-apk
        path: apps/native/src-tauri/gen/android/app/build/outputs/apk/**/*.apk
```

- Secrets go through `env:` rather than being interpolated into the script (the Tauri doc inlines `${{ secrets.* }}`; passing them as env vars avoids shell injection). Signing steps follow <https://raw.githubusercontent.com/tauri-apps/tauri-docs/v2/src/content/docs/distribute/Sign/android.mdx>.
- The Gradle `signingConfigs` block from that page reads `keystore.properties` unconditionally (`keystoreProperties["keyAlias"] as String`), so a PR build without the file fails at configuration time. Guard it (`if (keystorePropertiesFile.exists())` around the whole `create("release")` and the `signingConfig` assignment) so PR runs still compile an unsigned release APK.
- `tauri-action` has an `mobile: android` input, but it is "EXPERIMENTAL", does not install SDKs, and only swaps `build` for `android build`. Calling the CLI directly is as short and clearer. Source: <https://raw.githubusercontent.com/tauri-apps/tauri-action/dev/README.md>.

## 11. Changes This Implies for the Repo

1. Root `Cargo.toml`: `exclude = ["apps/native/src-tauri"]`.
2. `pnpm-workspace.yaml`: add `apps/native`.
3. `apps/web`: injectable `fetch` (plugin-http in native, `window.fetch` on web); runtime server URL screen and storage for native builds; Rsbuild `server.host` / `dev.client.host` from `TAURI_DEV_HOST`; ADR note on the `@tauri-apps/*` dependency exception.
4. `packages/middleware/src/cors.rs`: exact-match rule for `tauri://localhost`, `http://tauri.localhost`, `https://tauri.localhost` (needed only if any native code path uses the webview `fetch`, but cheap insurance), plus `docs/env.md`.
5. Android project: manifest entries for `RECORD_AUDIO`, `MODIFY_AUDIO_SETTINGS`, optional microphone feature; guarded release `signingConfigs`; keep `useHttpsScheme` false.
6. Linux: `with_webview` media-stream setting and a microphone-only `on_permission_request`.
7. CI: split `code-quality.yml` into a slim lint job and a standard Rust job; add `native-linux` (pinned `ubuntu-24.04`) and `native-android` (`ubuntu-latest`) gated by a slim path-filter job.

## 12. Open Items to Verify on the First Build

- Mixed-content behavior of a webview `fetch` from `tauri://localhost` to an `http://` LAN server on Linux (sources disagree; see Section 5).
- `getUserMedia` on Android from `http://tauri.localhost` (expected allowed as a secure context) and the runtime permission prompt with Tauri 2.12.
- Exact Linux `with_webview` code for `enable-media-stream`, and which GStreamer packages the `.deb` must list for capture and playback. (Settled in P5.3, see section 6: the code is in `apps/native/src-tauri/src/microphone.rs`, and the `.deb` needs no package beyond `libwebkit2gtk-4.1-0`'s own dependencies; a real microphone on both bundles is part of the P5.4 device test.)
- NDK 30 compatibility (until then, pin 29.x).
- Real sizes: `.deb`, `.rpm`, AppImage (with and without `bundleMediaFramework`), and the arm64 APK.
- Job durations on the chosen runners, to confirm the slim lint job stays well under 15 minutes.

## Verification

An independent re-check on 2026-11-26 opened every cited source again (curl against the live URLs, crates.io API, `npm view`, `git ls-remote`, Launchpad API) instead of trusting the citations.

| Claim | Result | What was checked |
| --- | --- | --- |
| `ubuntu-slim`: 1 vCPU, 5 GB RAM, 14 GB SSD, unprivileged container, 15-minute timeout, not for heavyweight builds | Confirmed | Runner tables (public and private) and the "Single-CPU runners" section of <https://docs.github.com/en/actions/reference/runners/github-hosted-runners>; the GA changelog repeats the 15-minute limit; $0.002/min on <https://docs.github.com/en/billing/reference/actions-runner-pricing> |
| `ubuntu-slim` has the Docker CLI but no engine, so no `services:` or `container:` jobs | Confirmed (the services part is an inference) | `install-docker-cli.sh` says "not the engine ... It cannot build or run containers locally"; the slim readme lists Docker Client, Compose, and Buildx only. GitHub does not state the service-container consequence for slim in so many words; it follows because the runner starts service and job containers through the local Docker engine. A secondary blog cites runner-images issue #13541 (maintainers unlikely to add `container:` support); that issue could not be opened from this environment |
| `ubuntu-latest` is Ubuntu 26.04 (rollout 2026-10-19 to 2026-11-19); pin `ubuntu-24.04` for glibc 2.39 vs 2.43 | Confirmed | Issue #14748 text ("beginning October 19, 2026", "complete the migration by November 19, 2026"; issue still open); runner-images README maps `ubuntu-latest` to Ubuntu 26.04; Launchpad `glibc` sources `2.39-0ubuntu8.12` (noble) and `2.43-2ubuntu2.7` (resolute = 26.04) |
| Origins: `tauri://localhost` on Linux/macOS, `http://tauri.localhost` on Windows/Android, `https` with `useHttpsScheme`, which blocks fetching `http` | Confirmed | config.rs lines 2213 to 2223 at `tauri-v2.12.7`; the unit test in `crates/tauri/src/manager/mod.rs` (lines 797 to 815) asserts exactly this mapping. Added the counterpoint that the same doc comment implies `tauri://localhost` allows mixed content |
| `tauri-plugin-http` requests run in Rust (no CORS, scope must match), bodies stream, port wildcards needed | Confirmed | guest-js `index.ts` doc comment and `ReadableStream` over `fetch_read_body`; <https://v2.tauri.app/reference/javascript/http/>; re-ran the plugin's `parse_url_pattern` against `urlpattern` 0.6.0 locally: `http://*` and `*://*` reject `http://192.168.1.10:3000/chats`, `http://*:*` accepts it, `https://*:*` accepts `https://example.com:8443/a` |
| Tauri CLI 2.12.6 Android template: SDK 37/37, default minSdk 24, `INTERNET`, cleartext only in debug | Confirmed | `app/build.gradle.kts` (`compileSdk = 37`, `targetSdk = 37`, `usesCleartextTraffic` `"false"` by default and `"true"` in `debug`), `AndroidManifest.xml`, `default_min_sdk_version()` returns 24 |
| wry Android client requests `RECORD_AUDIO` and `MODIFY_AUDIO_SETTINGS` at runtime | Confirmed, with a version correction | `RustWebChromeClient.kt` at `wry-v0.58.2`; Tauri 2.12.7 actually resolves wry 0.57.x, where the file is identical. Note updated in Sections 1 and 4 |
| Stable Tauri 2.12.7 (MSRV 1.90); 3.0.0-alpha.6 exists; tauri-action v1 dropped unstable Tauri | Partly corrected | crates.io: `max_stable_version` 2.12.7 (`rust_version` 1.90), `3.0.0-alpha.6` (1.95); npm `latest` 2.12.6, `next` 3.0.0-alpha.6; `action-v1.0.0` peels to `1deb371b...`. The tauri-action changelog only says it dropped "Tauri v1 and unstable v2 (alpha, beta, rc)", so Key Finding 1 now says that and marks 3.0 alpha support **UNVERIFIED** |

Other edits: added `windows-2025-vs2026` and `windows-11-vs2026-arm` to the runner catalog (both are on GitHub's runner page). Everything not listed above was not re-checked.
