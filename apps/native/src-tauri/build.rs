// The app's own commands get permissions like a plugin's, so the capability grants each one by
// name (`allow-save-file`) instead of every window getting every command.
fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(&["save_file"])),
    )
    .expect("the Tauri build script runs");
}
