//! Success envelopes. Domain payloads never sit at the JSON root, except health endpoints.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// `200` list response: `{ items, total }`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ListResponse<T> {
    pub items: Vec<T>,
    pub total: u64,
}

/// `200`/`201` details or action response: `{ data }`. Use `DetailsResponse<Option<T>>` when
/// absence is valid; it serializes as `{ data: null }`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct DetailsResponse<T> {
    pub data: T,
}

/// `200` cursor page, for lists that grow while the user reads them (chats): `{ items, next }`.
/// Pass `next` back as the `cursor` query parameter for the following page; it is `null` on the
/// last page. The cursor is opaque to clients.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct PageResponse<T> {
    pub items: Vec<T>,
    pub next: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_list_envelope() {
        let body = ListResponse {
            items: vec!["a"],
            total: 1,
        };
        assert_eq!(
            serde_json::to_value(body).expect("serializes"),
            serde_json::json!({ "items": ["a"], "total": 1 })
        );
    }

    #[test]
    fn serializes_the_last_page_with_null_next() {
        let body = PageResponse {
            items: vec!["a"],
            next: None,
        };
        assert_eq!(
            serde_json::to_value(body).expect("serializes"),
            serde_json::json!({ "items": ["a"], "next": null })
        );
    }

    #[test]
    fn serializes_nullable_details() {
        let body: DetailsResponse<Option<String>> = DetailsResponse { data: None };
        assert_eq!(
            serde_json::to_value(body).expect("serializes"),
            serde_json::json!({ "data": null })
        );
    }
}
