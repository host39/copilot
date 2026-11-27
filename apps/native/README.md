# Copilot Native Apps

The Linux desktop and Android apps: a [Tauri](https://v2.tauri.app/) 2.12 shell around the production build of `apps/web`. The app has no server of its own. On first launch it shows **Connect to Server**, checks `GET <url>/health`, stores the address on the device, and from then on sends every API call to that server through `tauri-plugin-http`. The design is in `docs/design/copilot.md` ("Native Apps"); the research behind it is `docs/research/native-and-ci.md`.

## Layout

| Path | What it is |
| --- | --- |
| `package.json` | `@copilot/native`: the Tauri CLI (`pnpm --filter @copilot/native tauri ...`) and the configuration tests |
| `src-tauri/Cargo.toml` | The `copilot-native` crate. It is excluded from the root Cargo workspace, so `make check` never needs WebKitGTK; it has its own `Cargo.lock` and `target/` |
| `src-tauri/tauri.conf.json` | Name (`Copilot`), identifier (`com.host39.copilot`), version (from `package.json`), the web build it bundles, the window, and the Content-Security-Policy |
| `src-tauri/capabilities/default.json` | Everything the page may ask of the shell (see [Security](#security)) |
| `src-tauri/src/lib.rs` | Registers the two plugins, `http` and `opener`, and the permission handler, and starts the app |
| `src-tauri/src/microphone.rs` | The microphone (see [Microphone](#microphone)): the permission answer and, on Linux, WebKitGTK's media stream setting, with unit tests |
| `src-tauri/icons/` | Generated from `apps/web/public/icons/icon.svg` |
| `src-tauri/gen/android/` | The Android Studio project from `tauri android init`, committed because it is customized (see [Android](#android)) |
| `test/config.test.ts` | Checks the rules below: identifier, CSP, capability scopes, plugin list, Android signing, insets, and permissions, and the microphone wiring |
| `test/workflow.test.ts` | Checks the CI workflow (see [CI](#ci)): triggers, runners, the bundle checks, and the uploads |

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
| `make native-test` | The shell's Rust unit tests (needs the Linux packages) |
| `pnpm --filter @copilot/native test` | The configuration tests (part of `make test`; no WebKitGTK needed) |

Loopback and private addresses (`localhost`, `10.x`, `172.16.x` to `172.31.x`, `192.168.x`, `100.64.x` to `100.127.x`, `fc00::/7`) connect over plain `http://` without a prompt. Any other `http://` address needs the user to confirm "Connect without encryption" first; use HTTPS through a reverse proxy for servers outside the local network. The sign-in screens show the server with a **Change** link back to the Connect screen; changing the server signs out.

## Build

| Command | Output |
| --- | --- |
| `make native-build` | `src-tauri/target/release/bundle/deb/Copilot_<version>_amd64.deb`. `pnpm --filter @copilot/native tauri build` without `--bundles` also builds the `.rpm` and the AppImage |
| `make android-build` | `src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release.apk`, for arm64 phones. Install it with `adb install -r <file>` or by opening it on the phone |

Build the `.deb` on the oldest distribution you want to support: it requires that system's glibc or newer. CI builds it on Ubuntu 24.04 (see [CI](#ci)).

The version comes from `version` in `package.json`; Android's version code is derived from it (`major * 1000000 + minor * 1000 + patch`), so bump it for every APK you hand out.

## CI

`.github/workflows/native.yml` ("Native apps") builds both apps on version tags (`v*`), on manual dispatch (Actions, Native apps, Run workflow), and on pushes to `main` and `claude/**` and pull requests that change `apps/native/**`, `apps/web/**`, `packages/contracts/**` (bundled by the web app), the workflow, or what the build reads from the root (`Makefile`, `package.json`, `pnpm-lock.yaml`, which pins the Tauri CLI and its JS packages, `pnpm-workspace.yaml`, `.nvmrc`, `rust-toolchain.toml`). Other changes skip the run. Each bundle is a workflow artifact of its own, named after the file and downloaded as that file (not zipped); tag builds keep them 90 days, other builds 14. Pull requests from forks build and check the bundles but upload none, so an installable file in the run list always comes from code in this repository.

| Job | Runner | Artifacts |
| --- | --- | --- |
| Linux bundles (`native-linux`) | `ubuntu-24.04`, pinned so the bundles need glibc 2.39 or newer, not the newest image's | `Copilot_<version>_amd64.deb`, `Copilot_<version>_amd64.AppImage` |
| Android APK (`native-android`) | `ubuntu-latest`, with the image's Android SDK, NDK 29, and Java 21 | `Copilot_<version>_arm64.apk` |

Before uploading, the Linux job runs `make native-lint` and `make native-test` and checks the `.deb`'s version, its WebKitGTK dependency, and the newest glibc symbol in the binary, and that the AppImage is an x86-64 type 2 AppImage. The Android job checks that the APK is signed with a debug certificate, holds native code for `arm64-v8a` only, carries the package name `com.host39.copilot` and the version from `package.json`, asks for `RECORD_AUDIO` and `MODIFY_AUDIO_SETTINGS` but not `CAMERA`, and turns off backup (`allowBackup` false and `dataExtractionRules` set in the compiled manifest). Each job writes the artifact sizes to its run summary. `test/workflow.test.ts` keeps the workflow's triggers, runners, and checks in place.

Each runner signs with a debug key of its own, so an APK from a later run does not install over one from an earlier run: uninstall first (see [Android](#android)).

## Security

- **CSP**: `app.security.csp` is the web app's policy (`CSP` in `apps/api/src/routes/web.rs`) with `connect-src ipc: http://ipc.localhost`. The page itself may load nothing from the network; API calls leave from Rust through the HTTP plugin, which neither CORS nor mixed-content rules nor Android's cleartext policy apply to. A test keeps the two policies in step.
- **Capability** (`capabilities/default.json`), for the `main` window only:
  - `http:default` scoped to `http://*:*` and `https://*:*`. The user picks the server at run time, so any host is allowed; the explicit port wildcard is needed, because `http://*` does not match `http://192.168.1.10:3000`. The web client sends the bearer token only to the stored server's origin.
  - `opener:allow-open-url` scoped to `http://*`, `https://*`, and `mailto:*`. The opener plugin opens links with `target="_blank"` (every link in a chat) in the system browser or mail app.
  - `core:app:default` and `core:app:allow-exit`, for Android's back button (`onBackButtonPress`) and leaving the app from the chat list.
  - Nothing else: no file system, shell, window, or event permissions. `build.removeUnusedCommands` strips the commands no capability allows.
- **Web view permissions**: the microphone only (see [Microphone](#microphone)). Every other request (camera, screen, location, notifications, MIDI, protected media) is denied.
- `useHttpsScheme` stays `false`. Changing it later moves the WebView's storage (the server address and the session token) on Android and Windows.

## Microphone

Push to talk (the composer's microphone, shown only for models that take audio) records with `getUserMedia({ audio })`. The shell lets the web view use the microphone and nothing else: `microphone::answer` in `src-tauri/src/microphone.rs` answers every permission request, allowing the microphone and denying every other kind.

| | Linux | Android |
| --- | --- | --- |
| Media support | WebKitGTK's `enable-media-stream`, set in `setup` right after the window is created, so the first page already has `navigator.mediaDevices`. WebKitGTK 2.52 turns it on by default; older releases documented it as off, so the shell sets it anyway | Built into the WebView |
| Permission | Granted at once, with no prompt: the page asks only after a tap on the microphone. Without the handler, WebKitGTK denies it | The manifest declares `RECORD_AUDIO` and `MODIFY_AUDIO_SETTINGS`; on the first tap, Android asks "Allow Copilot to record audio?". After "Don't allow", the page says the microphone is blocked; allow it again under Settings, Apps, Copilot, Permissions, Microphone |
| Audio capture | GStreamer's PulseAudio plugin (`gstreamer1.0-plugins-good`, which `libwebkit2gtk-4.1-0` already depends on; PipeWire desktops serve it through `pipewire-pulse`), so the `.deb` lists no extra package. The AppImage bundles WebKitGTK but not GStreamer's plugins (`bundleMediaFramework` is off), so it uses the system's | The system |

The manifest also marks the microphone as not required, so devices without one can install the app, and never declares `CAMERA`: the page records no video, and while `CAMERA` is undeclared, a photo for an `<input capture>` comes from the camera app without any permission.

## Android

The committed project differs from what `tauri android init` writes in four places. Re-apply them after regenerating it:

1. `app/build.gradle.kts`: the `release` build type is signed with the SDK's debug key (`signingConfig = signingConfigs.getByName("debug")`). The APK installs by sideloading without a release keystore; app stores refuse it. Each machine (and each CI runner) has its own debug key, so an APK built elsewhere cannot update an installed one in place: uninstall first, which also forgets the stored server and session. A release keystore replaces this before any store release.
2. `MainActivity.kt` pads the WebView's parent by the system bar, display cutout, and keyboard insets. Android 15 and later draw apps edge to edge, and the WebView does not offset the page by itself.
3. `AndroidManifest.xml` declares `RECORD_AUDIO` and `MODIFY_AUDIO_SETTINGS` and a microphone feature that is not required (see [Microphone](#microphone)).
4. `AndroidManifest.xml` turns off backup and device-to-device transfer: `android:allowBackup="false"` and `android:fullBackupContent="false"` (Android 11 and older), and `android:dataExtractionRules="@xml/data_extraction_rules"` (Android 12 and later), whose `res/xml/data_extraction_rules.xml` excludes every domain from `cloud-backup` and `device-transfer`. The WebView keeps its `localStorage` in the app's data folder, and that holds the session token and the server address; a backup restored on another phone would carry a token that still works. After a phone change, connect and sign in again.

Release builds keep `usesCleartextTraffic="false"` for the WebView. That does not affect `http://` servers on the LAN, because API calls go through the plugin, not the WebView.

Regenerating:

```bash
# Icons (also writes the Android launcher icons); then delete the ones the apps do not use
pnpm --filter @copilot/native tauri icon ../web/public/icons/icon.svg
rm -rf apps/native/src-tauri/icons/ios apps/native/src-tauri/icons/Square*Logo.png apps/native/src-tauri/icons/StoreLogo.png

# Android project (overwrites the four customizations above)
pnpm --filter @copilot/native tauri android init
```

## Changing the Identifier to `in.host39.copilot`

The app's identifier is `com.host39.copilot`. Tauri also turns the identifier into the Kotlin package of the Android project and into the JNI symbol names of the Rust library, and `in` is a Kotlin keyword, so `package in.host39.copilot` does not compile and setting `"identifier": "in.host39.copilot"` breaks the Android build. To publish the app as `in.host39.copilot`:

1. Leave `identifier` in `src-tauri/tauri.conf.json` at `com.host39.copilot`. It keeps naming the Kotlin package (`app/src/main/java/com/host39/copilot/`), the Gradle `namespace`, and the JNI bindings.
2. In `src-tauri/gen/android/app/build.gradle.kts`, change only the store-facing id: `applicationId = "in.host39.copilot"`. Android identifies the installed app, its data, and its store listing by the application id; the namespace is a build-time detail. The FileProvider authority follows it (`${applicationId}.fileprovider`).
3. Optionally, for the Linux app, add `src-tauri/tauri.linux.conf.json` with `{ "identifier": "in.host39.copilot" }`. It is merged only into Linux builds and names the desktop entry and the data folder (`~/.local/share/in.host39.copilot`).
4. Update the expectations in `test/config.test.ts`, then build the APK once (CI or `make android-build`) to confirm the Tauri CLI leaves the edited `applicationId` alone.

Do it before handing out the first build, if at all: a different application id is a different app to Android (no in-place update, its own data), and on Linux a different identifier moves the stored server and session.

## Device Test Checklist (P5.4)

The owner's checkpoint P5.4 covers what only a real device shows: section 12 of `docs/research/native-and-ci.md`, plus the Android file picker, camera capture, and back button. Note each result (pass, or what happened, with the device, the OS version, and the build) so an agent can file the fixes; `docs/design/tracks/p5-4.md` is the place for them.

### Setup

1. **Server.** On a computer on the same network as the phone, set `HOST=0.0.0.0` in `apps/api/.env.local`, then run `make dev-api` and, in another terminal, `make fake-llm`. Sign in as the administrator (the first account) and add the fake server in Settings, Providers: OpenAI-compatible, Custom, base URL `http://127.0.0.1:4010/v1`, key `fake-key`. Its `fake-audio` model takes and returns audio (`tools/fake-llm/README.md`); a real OpenAI-compatible audio model works too.
2. **Builds.** Download `Copilot_<version>_amd64.deb`, `Copilot_<version>_amd64.AppImage`, and `Copilot_<version>_arm64.apk` from the latest green "Native apps" run of a **push** to `main`, a `v*` tag, or one of your own `claude/...` branches (Actions, Native apps, filter by Event `push` and the branch, the run, Artifacts), never from a pull request's run, or build them (`make native-build`, `make android-build`). Runs for pull requests from forks upload no bundles, but filtering by event and branch makes sure the files came from code in this repository.
3. **Install.**
   - `.deb`: `sudo apt install ./Copilot_<version>_amd64.deb`, then start Copilot from the app menu.
   - AppImage: `chmod +x Copilot_<version>_amd64.AppImage` and run it.
   - APK: uninstall any earlier Copilot build first (each CI runner signs with its own key), then `adb install Copilot_<version>_arm64.apk`, or open the file on the phone and allow installs from that app.

### Both Platforms

- [ ] **Connect over plain HTTP.** On Connect to Server, enter `http://<the computer's LAN address>:3000`. Expect: no warning (a private address), sign-in works, the chat list loads, and a reply to `hello` streams in. On Linux this also settles the research note's open question about mixed content from `tauri://localhost`: the page itself makes no API calls (the HTTP plugin does).
- [ ] **Push to talk.** Open a chat with `fake-audio` and tap the microphone. Expect: on Android, the system asks "Allow Copilot to record audio?" the first time; on Linux, no prompt. The bar counts against the cap (`0:03 / 5:00`); Stop attaches the clip with a player that plays your voice back; the reply to it ends in "(with 1 attachment)".
- [ ] **Spoken replies.** Turn on Speak replies and send `audio`. Expect: a one-second tone plays while the reply streams, and the saved reply plays again from its player.
- [ ] **Links.** Send `see https://example.com` (the fake repeats it). Expect: tapping the link in the reply opens the system browser, and the app stays on the chat.
- [ ] **Saving a file.** Use the download button of an attachment chip or of a project file. Expect: the file lands in Downloads (or a save dialog opens). Note what happens either way: the page saves through an `<a download>` on a `blob:` URL, which WebKitGTK and Android's WebView may each handle differently or ignore.
- [ ] **Restart.** Quit and reopen the app. Expect: it opens on the chat list, still connected and signed in.

### Linux

- [ ] **Microphone on the first page.** Right after starting the app, before anything else, tap the microphone in a `fake-audio` chat. Expect: recording starts (not "This browser cannot use a microphone.").
- [ ] **Both bundles record.** Repeat push to talk with the `.deb` and with the AppImage. Expect: both record your voice. If the AppImage cannot start the microphone or records silence while the `.deb` works, the system's GStreamer plugins did not load in it: note it; the fix is `bundle > linux > appimage > bundleMediaFramework: true`, at the cost of a much larger AppImage.
- [ ] **GStreamer packages.** Only on a system where the `.deb` cannot record: note the distribution and the output of `gst-inspect-1.0 pulseaudio`. Ubuntu's `libwebkit2gtk-4.1-0` already pulls in `gstreamer1.0-plugins-good`, which holds the PulseAudio source that PipeWire desktops also serve.

### Android

- [ ] **Microphone denied.** Tap the microphone, choose "Don't allow" (or turn the permission off in the app's settings). Expect: "Microphone access is blocked. Allow the microphone for Copilot in Android's settings (Apps, Copilot, Permissions), then try again." After allowing it there, the next tap records.
- [ ] **File picker.** Tap Attach files in the composer and pick an image and a PDF together. Expect: Android's picker opens, allows several files, and both appear as chips with upload progress; the message sends. Repeat with Upload in a project's files.
- [ ] **Camera capture.** In the same picker, look for a way to take a photo. Expect today: none, because the attach input has no `capture` attribute and the WebView's picker offers only files; note whether yours does (some phones list a camera app). If a camera button is wanted, it is a small web change (an input with `accept="image/*" capture="environment"`), which wry sends to the camera app without asking for a camera permission.
- [ ] **Back button.** Expect, in this order of precedence: with a dialog open (for example Rename or Delete), back closes it; after moving between pages, back returns to the previous one; on a page opened directly (no history), back goes to the chat list; on the chat list, back leaves the app.
- [ ] **Edge to edge and the keyboard.** Expect: the header sits below the status bar and the camera cutout, nothing hides under the navigation bar, and the composer stays above the keyboard while typing, in portrait and in landscape.
- [ ] **Background and return.** Send `slow long`, switch to another app, and come back after a few seconds. Expect: the reply has continued (or finished) and keeps streaming.
