//! The model list (`GET {base}/v1/models?limit=1000`, paged by `after_id` while `has_more`) and
//! the capabilities the Models API reports for each model.
//!
//! | Field | Reported as |
//! | --- | --- |
//! | `display_name` | display name |
//! | `max_input_tokens` | context window |
//! | `max_tokens` | max output tokens |
//! | `capabilities.image_input.supported` | `vision` |
//! | `capabilities.pdf_input.supported` | `pdf` |
//! | `capabilities.thinking.types.adaptive.supported` | `reasoning` (the adapter sends adaptive thinking only) |
//! | `capabilities.thinking.types.disabled.supported` | the `off` level |
//! | `capabilities.effort.{low,medium,high}.supported` | the `low`, `medium`, and `high` levels |
//!
//! Claude models take and return no audio, so `audio_in` and `audio_out` are reported off (core
//! locks them too); tool use is not reported, so core's default (on) applies. Retired models are
//! left out. Fields that are missing or of another type read as "not reported".

use copilot_core::shared::constants::MAX_MODELS_PER_PROVIDER;
use copilot_core::{
    DomainResult, Outbound, ProviderTarget, ReasoningLevel, RemoteModel, ReportedCapabilities,
};
use reqwest::{Client, Url};
use serde_json::Value;

use crate::anthropic::{endpoint, headers};
use crate::http::{bad_response, blocked_error, body, client, send};

/// Models asked for per page (the API's maximum).
const PAGE_SIZE: &str = "1000";
/// Pages read at most: enough to pass `MAX_MODELS_PER_PROVIDER`, so core can refuse a longer
/// list instead of the adapter cutting it short.
const PAGES_MAX: usize = MAX_MODELS_PER_PROVIDER / 1_000 + 1;

/// One page of the list.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Page {
    pub(crate) models: Vec<RemoteModel>,
    /// The `after_id` of the next page, when there is one.
    pub(crate) next: Option<String>,
}

/// The connection's models with what the provider reports about them.
pub(crate) async fn list(
    clients: &Outbound<Client>,
    target: &ProviderTarget,
) -> DomainResult<Vec<RemoteModel>> {
    let client = client(clients, target)?;
    let mut models = Vec::new();
    let mut after: Option<String> = None;
    for _ in 0..PAGES_MAX {
        let mut url = Url::parse(&endpoint(target.base_url.as_str(), "models"))
            .map_err(|_| blocked_error())?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("limit", PAGE_SIZE);
            if let Some(id) = &after {
                query.append_pair("after_id", id);
            }
        }
        let response = send(target, headers(target, client.get(url))?).await?;
        let bytes = body(target, response).await?;
        let page = parse(&bytes).ok_or_else(|| {
            tracing::warn!(provider_id = %target.provider_id, "provider.models.invalid");
            bad_response()
        })?;
        models.extend(page.models);
        match page.next {
            Some(id) if models.len() <= MAX_MODELS_PER_PROVIDER && after.as_ref() != Some(&id) => {
                after = Some(id);
            }
            _ => break,
        }
    }
    Ok(models)
}

/// One page, or `None` when the body is not a model list.
pub(crate) fn parse(bytes: &[u8]) -> Option<Page> {
    let page: Value = serde_json::from_slice(bytes).ok()?;
    let items = page.get("data")?.as_array()?;
    let more = page.get("has_more").and_then(Value::as_bool) == Some(true);
    let next = page
        .get("last_id")
        .and_then(Value::as_str)
        .filter(|id| more && !id.is_empty())
        .map(str::to_owned);
    Some(Page {
        models: items.iter().filter_map(model).collect(),
        next,
    })
}

/// One listed model; entries without a string id, and retired models, are skipped.
fn model(item: &Value) -> Option<RemoteModel> {
    let remote_id = item.get("id")?.as_str()?;
    if item.get("lifecycle").and_then(Value::as_str) == Some("retired") {
        return None;
    }
    let display_name = item
        .get("display_name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(remote_id);
    let capabilities = item.get("capabilities");
    let flag = |path: &[&str]| supported(capabilities, path);
    let adaptive = flag(&["thinking", "types", "adaptive"]);
    let mut reported = ReportedCapabilities {
        vision: flag(&["image_input"]),
        pdf: flag(&["pdf_input"]),
        audio_in: Some(false),
        audio_out: Some(false),
        reasoning: adaptive,
        ..ReportedCapabilities::default()
    };
    if adaptive == Some(true) {
        let mut levels = Vec::new();
        if flag(&["thinking", "types", "disabled"]) == Some(true) {
            levels.push(ReasoningLevel::Off);
        }
        if flag(&["effort"]) == Some(true) {
            for (level, name) in [
                (ReasoningLevel::Low, "low"),
                (ReasoningLevel::Medium, "medium"),
                (ReasoningLevel::High, "high"),
            ] {
                if flag(&["effort", name]) == Some(true) {
                    levels.push(level);
                }
            }
        }
        // Only `off` (no effort reported) would leave nothing to turn on: keep core's default.
        reported.reasoning_levels = levels
            .iter()
            .any(|level| *level != ReasoningLevel::Off)
            .then_some(levels);
    }
    Some(RemoteModel {
        remote_id: remote_id.to_owned(),
        display_name: display_name.to_owned(),
        context_window: count(item, "max_input_tokens"),
        max_output_tokens: count(item, "max_tokens"),
        reported,
    })
}

/// `supported` at the end of `path` under `capabilities`.
fn supported(capabilities: Option<&Value>, path: &[&str]) -> Option<bool> {
    let mut node = capabilities?;
    for key in path {
        node = node.get(*key)?;
    }
    node.get("supported")?.as_bool()
}

/// A positive token count that fits a `u32`.
fn count(item: &Value, field: &str) -> Option<u32> {
    item.get(field)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn listed(capabilities: Value) -> Value {
        json!({
            "type": "model",
            "id": "claude-example-1",
            "display_name": "Claude Example 1",
            "created_at": "2026-01-01T00:00:00Z",
            "lifecycle": "active",
            "max_input_tokens": 1_000_000,
            "max_tokens": 128_000,
            "capabilities": capabilities,
        })
    }

    fn page(items: Vec<Value>, has_more: bool) -> Vec<u8> {
        let last = items
            .last()
            .and_then(|item| item.get("id"))
            .cloned()
            .unwrap_or(Value::Null);
        json!({ "data": items, "has_more": has_more, "first_id": null, "last_id": last })
            .to_string()
            .into_bytes()
    }

    #[test]
    fn reads_reported_capabilities_and_limits() {
        let item = listed(json!({
            "image_input": { "supported": true },
            "pdf_input": { "supported": true },
            "thinking": {
                "supported": true,
                "types": {
                    "adaptive": { "supported": true },
                    "enabled": { "supported": false },
                    "disabled": { "supported": false },
                },
            },
            "effort": {
                "supported": true,
                "low": { "supported": true },
                "medium": { "supported": true },
                "high": { "supported": true },
                "xhigh": { "supported": true },
                "max": { "supported": true },
            },
        }));
        let parsed = parse(&page(vec![item], false)).expect("a page");
        assert_eq!(parsed.next, None);
        assert_eq!(
            parsed.models,
            [RemoteModel {
                remote_id: "claude-example-1".to_owned(),
                display_name: "Claude Example 1".to_owned(),
                context_window: Some(1_000_000),
                max_output_tokens: Some(128_000),
                reported: ReportedCapabilities {
                    vision: Some(true),
                    pdf: Some(true),
                    tools: None,
                    audio_in: Some(false),
                    audio_out: Some(false),
                    reasoning: Some(true),
                    reasoning_levels: Some(vec![
                        ReasoningLevel::Low,
                        ReasoningLevel::Medium,
                        ReasoningLevel::High,
                    ]),
                    default_reasoning: None,
                },
            }]
        );
    }

    #[test]
    fn offers_off_only_where_thinking_can_be_disabled() {
        let item = listed(json!({
            "thinking": { "supported": true, "types": {
                "adaptive": { "supported": true },
                "disabled": { "supported": true },
            } },
            "effort": { "supported": true, "low": { "supported": true }, "medium": { "supported": false }, "high": { "supported": true } },
        }));
        let model = &parse(&page(vec![item], false)).expect("a page").models[0];
        assert_eq!(
            model.reported.reasoning_levels,
            Some(vec![
                ReasoningLevel::Off,
                ReasoningLevel::Low,
                ReasoningLevel::High
            ])
        );
    }

    #[test]
    fn a_model_without_adaptive_thinking_reports_no_reasoning() {
        let item = listed(json!({
            "image_input": { "supported": false },
            "thinking": { "supported": true, "types": {
                "adaptive": { "supported": false },
                "enabled": { "supported": true },
            } },
        }));
        let model = &parse(&page(vec![item], false)).expect("a page").models[0];
        assert_eq!(model.reported.vision, Some(false));
        assert_eq!(model.reported.pdf, None);
        assert_eq!(model.reported.reasoning, Some(false));
        assert_eq!(model.reported.reasoning_levels, None);

        let bare = json!({ "id": "claude-bare", "max_tokens": "many" });
        let model = &parse(&page(vec![bare], false)).expect("a page").models[0];
        assert_eq!(model.display_name, "claude-bare");
        assert_eq!(model.max_output_tokens, None);
        assert_eq!(model.reported.reasoning, None);
        assert_eq!(model.reported.audio_in, Some(false));
    }

    #[test]
    fn pages_while_has_more_and_skips_retired_and_unnamed_entries() {
        let mut retired = listed(json!({}));
        retired["id"] = json!("claude-old");
        retired["lifecycle"] = json!("retired");
        let items = vec![
            json!({ "display_name": "No id" }),
            retired,
            listed(json!({})),
        ];
        let parsed = parse(&page(items, true)).expect("a page");
        assert_eq!(parsed.next.as_deref(), Some("claude-example-1"));
        assert_eq!(parsed.models.len(), 1);
        assert_eq!(parsed.models[0].remote_id, "claude-example-1");

        assert_eq!(parse(b"{\"models\": []}"), None);
        assert_eq!(parse(b"not json"), None);
    }
}
