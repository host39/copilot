//! Decrypted secret values.

use std::fmt;

/// What `Debug` prints in place of a secret value.
pub const REDACTED: &str = "***";

/// A decrypted provider key or tool server header map.
///
/// `Debug` prints `***` and there is no `Display`, so a secret cannot end up in a log line or an
/// error message by accident. Read the value only where it is sent (`expose`, `text`).
#[derive(Clone, Eq, PartialEq)]
pub struct Secret(Vec<u8>);

impl Secret {
    pub fn new(value: impl Into<Vec<u8>>) -> Self {
        Self(value.into())
    }

    /// The raw bytes, for sealing or for a request header.
    pub fn expose(&self) -> &[u8] {
        &self.0
    }

    /// The value as UTF-8 text, or `None` when it is not valid UTF-8.
    pub fn text(&self) -> Option<&str> {
        std::str::from_utf8(&self.0).ok()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(REDACTED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_prints_the_value() {
        let secret = Secret::new("sk-live-123");
        assert_eq!(format!("{secret:?}"), "***");
        assert_eq!(format!("{:?}", Some(&secret)), "Some(***)");
    }

    #[test]
    fn exposes_bytes_and_text() {
        let secret = Secret::new("sk-live-123");
        assert_eq!(secret.expose(), b"sk-live-123");
        assert_eq!(secret.text(), Some("sk-live-123"));
        assert_eq!(Secret::new(vec![0xff, 0xfe]).text(), None);
    }
}
