//! `PATCH` bodies: an absent field leaves the value unchanged. Fields that can be cleared are
//! `Option<Option<T>>` read with [`nullable`], so `null` clears them.

use serde::{Deserialize, Deserializer};

/// Reads a field that `null` clears. Use it with `#[serde(default)]`: an absent field gives
/// `None`, `null` gives `Some(None)`, and a value gives `Some(Some(value))`.
pub(crate) fn nullable<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq)]
    struct Body {
        #[serde(default, deserialize_with = "crate::patch::nullable")]
        value: Option<Option<u32>>,
    }

    fn read(json: serde_json::Value) -> Option<Option<u32>> {
        serde_json::from_value::<Body>(json)
            .expect("valid body")
            .value
    }

    #[test]
    fn tells_absent_null_and_value_apart() {
        assert_eq!(read(serde_json::json!({})), None);
        assert_eq!(read(serde_json::json!({ "value": null })), Some(None));
        assert_eq!(read(serde_json::json!({ "value": 7 })), Some(Some(7)));
    }
}
