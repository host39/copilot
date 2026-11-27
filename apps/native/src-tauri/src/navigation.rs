//! The navigation guard: the main web view only ever shows the bundled app. A navigation to any
//! other origin is cancelled, and a web link among them opens in the system browser instead (the
//! opener plugin already does this for links with `target="_blank"`). With navigation pinned to
//! the bundle, the microphone answer (`microphone.rs`), which does not look at the page's URL, can
//! only ever reach the app.

use tauri::plugin::{Builder, TauriPlugin};
use tauri::{Manager, Runtime, Url};
use tauri_plugin_opener::OpenerExt;

/// Whether the main web view may show `url`: the bundle's origin (`tauri://localhost` on Linux,
/// `http://tauri.localhost` on Android with the default http scheme), or the dev server's origin
/// (`dev`) under `tauri dev`.
pub fn allowed(url: &Url, dev: Option<&Url>) -> bool {
    let bundle = match url.scheme() {
        "tauri" => url.host_str() == Some("localhost") && url.port().is_none(),
        "http" => url.host_str() == Some("tauri.localhost") && url.port().is_none(),
        _ => false,
    };
    bundle || dev.is_some_and(|dev| dev.origin() == url.origin())
}

/// Whether a refused URL is a web link to hand to the system browser.
pub fn external(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
}

/// The plugin that guards every navigation of every web view.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("navigation")
        .on_navigation(|webview, url| {
            let dev = if tauri::is_dev() {
                webview.app_handle().config().build.dev_url.clone()
            } else {
                None
            };
            if allowed(url, dev.as_ref()) {
                return true;
            }
            // Nothing is logged: the shell installs no logger, and a URL may carry a token.
            if external(url) {
                // A failure leaves the link unopened; the page stays where it was either way.
                let _ = webview.opener().open_url(url.as_str(), None::<&str>);
            }
            false
        })
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(value: &str) -> Url {
        Url::parse(value).expect("a test URL parses")
    }

    #[test]
    fn allows_the_bundle_on_linux_and_android() {
        assert!(allowed(&url("tauri://localhost/"), None));
        assert!(allowed(
            &url("tauri://localhost/chats/1?x=1#message-2"),
            None
        ));
        assert!(allowed(&url("http://tauri.localhost/settings"), None));
    }

    #[test]
    fn refuses_every_other_origin() {
        for value in [
            "https://example.com/",
            "http://localhost:4000/",
            "https://tauri.localhost/",
            "http://tauri.localhost:8080/",
            "tauri://localhost:1/",
            "tauri://evil/",
            "http://tauri.localhost.example.com/",
            "file:///etc/passwd",
            "data:text/html,<p>hi</p>",
            "blob:tauri://localhost/0b1c",
            "about:blank",
            "javascript:alert(1)",
        ] {
            assert!(!allowed(&url(value), None), "{value}");
        }
    }

    #[test]
    fn allows_the_dev_server_only_when_given() {
        let dev = url("http://localhost:4000");
        assert!(allowed(&url("http://localhost:4000/chats/1"), Some(&dev)));
        assert!(!allowed(&url("http://localhost:4001/"), Some(&dev)));
        assert!(!allowed(&url("https://localhost:4000/"), Some(&dev)));
        assert!(!allowed(&url("http://localhost:4000/"), None));
    }

    #[test]
    fn hands_only_web_links_to_the_browser() {
        assert!(external(&url("https://example.com/")));
        assert!(external(&url("http://192.168.1.10:3000/")));
        assert!(!external(&url("file:///etc/passwd")));
        assert!(!external(&url("mailto:someone@example.com")));
        assert!(!external(&url("javascript:alert(1)")));
    }
}
