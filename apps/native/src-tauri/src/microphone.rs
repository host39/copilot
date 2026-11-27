//! The microphone for push to talk (`apps/web/src/audio/recorder.ts` calls
//! `getUserMedia({ audio })`). The web view may use the microphone and nothing else. Background:
//! `docs/research/native-and-ci.md`, sections 4 and 6.

use tauri::webview::{PermissionKind, PermissionResponse};

/// Answers the web view's permission requests: the microphone is allowed, everything else is
/// denied.
///
/// - Linux: WebKitGTK grants the microphone at once (the page asks only after a tap on the
///   microphone button). Unanswered, WebKitGTK would deny it.
/// - Android: an allowed microphone request still goes through Android's runtime prompt for
///   `RECORD_AUDIO` and `MODIFY_AUDIO_SETTINGS`, which the manifest declares. Camera, protected
///   media, and MIDI requests are denied without a prompt.
pub fn answer(kind: PermissionKind) -> PermissionResponse {
    match kind {
        PermissionKind::Microphone => PermissionResponse::Allow,
        _ => PermissionResponse::Deny,
    }
}

/// Turns on WebKitGTK's media streams, which are off by default (wry leaves them off): without
/// them the page has no `navigator.mediaDevices` and cannot record.
///
/// Runs in `setup`, on the main thread, right after Tauri creates the windows of
/// `tauri.conf.json`. There `with_webview` runs at once, so the setting reaches the web process
/// before the first page commits.
#[cfg(target_os = "linux")]
pub fn enable<R: tauri::Runtime>(app: &tauri::App<R>) -> tauri::Result<()> {
    use tauri::Manager;
    use webkit2gtk::{SettingsExt, WebViewExt};

    for window in app.webview_windows().values() {
        window.with_webview(|webview| {
            if let Some(settings) = WebViewExt::settings(&webview.inner()) {
                settings.set_enable_media_stream(true);
            }
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_the_microphone() {
        assert_eq!(
            answer(PermissionKind::Microphone),
            PermissionResponse::Allow
        );
    }

    #[test]
    fn denies_everything_else() {
        for kind in [
            PermissionKind::Camera,
            PermissionKind::DisplayCapture,
            PermissionKind::Geolocation,
            PermissionKind::Notifications,
            PermissionKind::ClipboardRead,
            PermissionKind::Midi,
            PermissionKind::Sensors,
            PermissionKind::MediaKeySystemAccess,
            PermissionKind::LocalFonts,
            PermissionKind::WindowManagement,
            PermissionKind::PointerLock,
            PermissionKind::AutomaticDownloads,
            PermissionKind::FileSystemAccess,
            PermissionKind::Autoplay,
            PermissionKind::Other,
        ] {
            assert_eq!(answer(kind), PermissionResponse::Deny, "{kind}");
        }
    }
}
