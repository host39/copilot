//! Saving a file the web app downloaded (a chat attachment, a project file, a tool result image).
//! Web views ignore `<a download>` on a `blob:` URL without a download handler, so native builds
//! hand the bytes to this command instead (`apps/web/src/components/Files/save.ts`): the system's
//! save dialog asks where (Android's document picker returns a `content://` URI), and the bytes are
//! written there. The web view gets no file system access of its own.

use std::io::Write;

use percent_encoding::percent_decode_str;
use tauri::ipc::{InvokeBody, Request};
use tauri::{AppHandle, Runtime};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_fs::{FsExt, OpenOptions};

/// The request header with the file's suggested name, percent-encoded (`encodeURIComponent`).
pub const NAME_HEADER: &str = "x-file-name";

/// The name when the request gives none.
const FALLBACK_NAME: &str = "download";

/// The suggested name from the header: decoded, without any folder part or control characters,
/// and never empty.
pub fn file_name(header: Option<&str>) -> String {
    let decoded = header
        .map(|value| percent_decode_str(value).decode_utf8_lossy().into_owned())
        .unwrap_or_default();
    let base = decoded.rsplit(['/', '\\']).next().unwrap_or_default();
    let clean: String = base.chars().filter(|c| !c.is_control()).collect();
    let clean = clean.trim();
    if clean.is_empty() || clean == "." || clean == ".." {
        FALLBACK_NAME.to_owned()
    } else {
        clean.to_owned()
    }
}

/// Asks where to save the request's bytes and writes them there. `false` when the user cancelled.
#[tauri::command]
pub async fn save_file<R: Runtime>(
    app: AppHandle<R>,
    request: Request<'_>,
) -> Result<bool, String> {
    let InvokeBody::Raw(bytes) = request.body() else {
        return Err("The file's bytes are missing.".to_owned());
    };
    let header = request
        .headers()
        .get(NAME_HEADER)
        .and_then(|value| value.to_str().ok());
    let name = file_name(header);
    // The dialog blocks until the user answers, so it waits off the async runtime's threads.
    let dialog = app.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        dialog
            .dialog()
            .file()
            .set_file_name(name)
            .blocking_save_file()
    })
    .await
    .map_err(|error| error.to_string())?;
    let Some(path) = picked else {
        return Ok(false);
    };
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    let mut file = app
        .fs()
        .open(path, options)
        .map_err(|error| error.to_string())?;
    file.write_all(bytes).map_err(|error| error.to_string())?;
    file.flush().map_err(|error| error.to_string())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_the_suggested_name() {
        assert_eq!(file_name(Some("notes.md")), "notes.md");
        assert_eq!(
            file_name(Some("Trip%20plans%20%E2%9C%88.pdf")),
            "Trip plans \u{2708}.pdf"
        );
    }

    #[test]
    fn keeps_only_the_last_part_of_a_path() {
        assert_eq!(file_name(Some("..%2F..%2F.bashrc")), ".bashrc");
        assert_eq!(file_name(Some("C%3A%5CUsers%5Cme%5Cpic.png")), "pic.png");
    }

    #[test]
    fn falls_back_for_an_empty_or_unusable_name() {
        for header in [
            None,
            Some(""),
            Some("%20"),
            Some("a%2F"),
            Some(".."),
            Some("%0A%0D"),
        ] {
            assert_eq!(file_name(header), FALLBACK_NAME, "{header:?}");
        }
    }

    #[test]
    fn drops_control_characters() {
        assert_eq!(file_name(Some("re%0Aport%00.txt")), "report.txt");
    }
}
