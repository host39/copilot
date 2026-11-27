//! The Copilot desktop and Android app: one window with the web app's production build. API calls
//! go through the HTTP plugin (`apps/web/src/native.ts`), and links in chats open in the system
//! browser through the opener plugin. The plugins' scopes are in `capabilities/default.json`. The
//! window only ever shows the bundled app (`navigation.rs`), may use the microphone and nothing
//! else (`microphone.rs`), and saves downloads through the system's save dialog (`files.rs`).

mod files;
mod microphone;
mod navigation;

/// Starts the app; the desktop `main` and the Android activity both call it.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(navigation::init())
        .invoke_handler(tauri::generate_handler![files::save_file])
        .on_permission_request(|_, kind| microphone::answer(kind))
        .setup(|_app| {
            #[cfg(target_os = "linux")]
            microphone::enable(_app)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("the Tauri runtime starts with the bundled configuration");
}
