//! The Copilot desktop and Android app: one window with the web app's production build. API calls
//! go through the HTTP plugin (`apps/web/src/native.ts`), and links in chats open in the system
//! browser through the opener plugin. The plugins' scopes are in `capabilities/default.json`.

/// Starts the app; the desktop `main` and the Android activity both call it.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_opener::init())
        .run(tauri::generate_context!())
        .expect("the Tauri runtime starts with the bundled configuration");
}
