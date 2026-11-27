//! What Google's native model list adds to a `gemini` connection's models.
//!
//! The OpenAI-compatible list (`GET {base}/models`, base `.../v1beta/openai`) names each model
//! and nothing more. Google's native list next to it (`GET .../v1beta/models`, the base without
//! its `/openai`) reports, per model `models/<id>`:
//!
//! | Field | Reported as |
//! | --- | --- |
//! | `inputTokenLimit` | context window |
//! | `outputTokenLimit` | max output tokens |
//! | `thinking` | `reasoning` (only when it is a boolean) |
//! | `displayName` | display name, when the OpenAI-compatible list gave none |
//!
//! The key goes in `x-goog-api-key` (the native API reads no bearer key). The list is read in
//! pages of [`PAGE_SIZE`], at most [`PAGES_MAX`] of them; a failure keeps the models as the
//! OpenAI-compatible list named them, so the user sets the limits by hand as before.

use std::collections::HashMap;

use copilot_core::{DomainResult, ProviderTarget, RemoteModel};
use reqwest::header::HeaderName;
use reqwest::{Client, Url};
use serde_json::Value;

use crate::http::{api_key, bad_response, body, send};
use crate::openai::models::count;

/// Models per page of the native list (Google's largest page).
pub(crate) const PAGE_SIZE: u32 = 1_000;

/// Pages read at most: more models than a connection keeps (`MAX_MODELS_PER_PROVIDER`), and a
/// bound on a list that keeps naming a next page.
pub(crate) const PAGES_MAX: usize = 3;

/// The header Google's native API takes the key in.
const KEY_HEADER: HeaderName = HeaderName::from_static("x-goog-api-key");

/// What the native list says about one model.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Native {
    pub display_name: Option<String>,
    pub input: Option<u32>,
    pub output: Option<u32>,
    pub thinking: Option<bool>,
}

/// One page of the native list: each model by its id without `models/`, and the next page's
/// token.
#[derive(Debug, Default, Eq, PartialEq)]
pub(crate) struct Page {
    pub models: HashMap<String, Native>,
    pub next: Option<String>,
}

/// The models with what the native list reports about them; a failure keeps them as listed.
pub(crate) async fn add_native(
    client: &Client,
    target: &ProviderTarget,
    models: Vec<RemoteModel>,
) -> Vec<RemoteModel> {
    let Some(root) = native_root(target) else {
        return models;
    };
    match native(client, target, &root).await {
        Ok(found) => models
            .into_iter()
            .map(|model| match found.get(&model.remote_id) {
                Some(native) => apply(model, native),
                None => model,
            })
            .collect(),
        Err(error) => {
            tracing::warn!(
                provider_id = %target.provider_id,
                code = error.code(),
                "provider.models.details-failed"
            );
            models
        }
    }
}

/// The native API root: the base URL without its `/openai`, or `None` for another path.
fn native_root(target: &ProviderTarget) -> Option<String> {
    target
        .base_url
        .as_str()
        .strip_suffix("/openai")
        .map(str::to_owned)
}

/// Every model of the native list, read page by page.
async fn native(
    client: &Client,
    target: &ProviderTarget,
    root: &str,
) -> DomainResult<HashMap<String, Native>> {
    let mut found = HashMap::new();
    let mut token: Option<String> = None;
    for _ in 0..PAGES_MAX {
        let mut url = Url::parse(&format!("{root}/models")).map_err(|_| bad_response())?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("pageSize", &PAGE_SIZE.to_string());
            if let Some(token) = &token {
                query.append_pair("pageToken", token);
            }
        }
        let mut request = client.get(url);
        if let Some(value) = api_key(target)? {
            request = request.header(KEY_HEADER, value);
        }
        let response = send(target, request).await?;
        let bytes = body(target, response).await?;
        let page = parse(&bytes).ok_or_else(bad_response)?;
        found.extend(page.models);
        match page.next {
            Some(next) => token = Some(next),
            None => break,
        }
    }
    Ok(found)
}

/// One page of the native list, or `None` when the body is not one. Entries without a string
/// `name` are skipped; a page without `models` is empty.
pub(crate) fn parse(bytes: &[u8]) -> Option<Page> {
    let Value::Object(page) = serde_json::from_slice::<Value>(bytes).ok()? else {
        return None;
    };
    let items = match page.get("models") {
        None => &[][..],
        Some(Value::Array(items)) => items.as_slice(),
        Some(_) => return None,
    };
    let models = items
        .iter()
        .filter_map(|item| {
            let name = item.get("name")?.as_str()?;
            let id = name.strip_prefix("models/").unwrap_or(name);
            let native = Native {
                display_name: item
                    .get("displayName")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(str::to_owned),
                input: count(item, &["inputTokenLimit"]),
                output: count(item, &["outputTokenLimit"]),
                thinking: item.get("thinking").and_then(Value::as_bool),
            };
            Some((id.to_owned(), native))
        })
        .collect();
    let next = page
        .get("nextPageToken")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .map(str::to_owned);
    Some(Page { models, next })
}

/// The model with the native list's limits, thinking flag, and (when it has none of its own)
/// display name.
pub(crate) fn apply(mut model: RemoteModel, native: &Native) -> RemoteModel {
    if native.input.is_some() {
        model.context_window = native.input;
    }
    if native.output.is_some() {
        model.max_output_tokens = native.output;
    }
    if model.reported.reasoning.is_none() {
        model.reported.reasoning = native.thinking;
    }
    if let Some(name) = &native.display_name
        && model.display_name == model.remote_id
    {
        model.display_name.clone_from(name);
    }
    model
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn page(body: &Value) -> Page {
        parse(body.to_string().as_bytes()).expect("a page")
    }

    #[test]
    fn reads_limits_names_and_thinking() {
        let page = page(&json!({
            "models": [
                {
                    "name": "models/gemini-4-hydrogen",
                    "baseModelId": "gemini-4-hydrogen",
                    "version": "001",
                    "displayName": "Gemini 4 Hydrogen",
                    "inputTokenLimit": 1_048_576,
                    "outputTokenLimit": 65_536,
                    "supportedGenerationMethods": ["generateContent", "countTokens"],
                    "thinking": true,
                },
                { "name": "models/text-embedding-9", "inputTokenLimit": 2_048, "outputTokenLimit": 1 },
                { "displayName": "No name" },
                { "name": 7 },
            ],
            "nextPageToken": "page-2",
        }));
        assert_eq!(
            page.models.len(),
            2,
            "entries without a string name are skipped"
        );
        assert_eq!(
            page.models["gemini-4-hydrogen"],
            Native {
                display_name: Some("Gemini 4 Hydrogen".to_owned()),
                input: Some(1_048_576),
                output: Some(65_536),
                thinking: Some(true),
            }
        );
        assert_eq!(
            page.models["text-embedding-9"],
            Native {
                input: Some(2_048),
                output: Some(1),
                ..Native::default()
            }
        );
        assert_eq!(page.next.as_deref(), Some("page-2"));
    }

    #[test]
    fn reads_the_last_and_empty_pages() {
        assert_eq!(page(&json!({})), Page::default(), "no models, no next page");
        let last = page(&json!({ "models": [], "nextPageToken": "" }));
        assert_eq!(last.next, None, "an empty token ends the list");
        let odd = page(&json!({ "models": [{
            "name": "gemini-x",
            "displayName": "  ",
            "inputTokenLimit": 99_999_999_999_u64,
            "outputTokenLimit": 0,
            "thinking": "yes",
        }] }));
        assert_eq!(
            odd.models["gemini-x"],
            Native::default(),
            "out of range, zero, a blank name, and a thinking flag that is not a boolean"
        );
    }

    #[test]
    fn refuses_bodies_that_are_not_pages() {
        for body in ["<html>", "[]", r#"{"models":{}}"#, "42"] {
            assert!(parse(body.as_bytes()).is_none(), "{body}");
        }
    }

    #[test]
    fn applies_what_the_native_list_reports() {
        let listed = RemoteModel {
            remote_id: "gemini-4-hydrogen".to_owned(),
            display_name: "gemini-4-hydrogen".to_owned(),
            ..RemoteModel::default()
        };
        let native = Native {
            display_name: Some("Gemini 4 Hydrogen".to_owned()),
            input: Some(1_048_576),
            output: Some(65_536),
            thinking: Some(false),
        };
        let model = apply(listed.clone(), &native);
        assert_eq!(model.display_name, "Gemini 4 Hydrogen");
        assert_eq!(model.context_window, Some(1_048_576));
        assert_eq!(model.max_output_tokens, Some(65_536));
        assert_eq!(model.reported.reasoning, Some(false));

        let named = RemoteModel {
            display_name: "Hydrogen (listed)".to_owned(),
            context_window: Some(32_768),
            ..listed.clone()
        };
        let kept = apply(named, &Native::default());
        assert_eq!(
            kept.display_name, "Hydrogen (listed)",
            "a listed name stays"
        );
        assert_eq!(
            kept.context_window,
            Some(32_768),
            "nothing reported, nothing changes"
        );
        assert_eq!(kept.reported.reasoning, None);
    }
}
