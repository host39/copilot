//! OAuth requests on the hardened clients: one client per URL and network scope
//! (`Outbound::pick`, so the SSRF rules and the private-network policy apply as for the server
//! URL), no redirects, a time limit, and a size cap on every answer.

use std::time::Duration;

use bytes::Bytes;
use copilot_core::shared::constants::{OAUTH_RESPONSE_MAX_BYTES, OAUTH_TIMEOUT_SECS};
use copilot_core::{Blocked, NetworkScope, Outbound, read_capped};
use reqwest::header::{ACCEPT, HeaderMap};
use reqwest::{Client, RequestBuilder};
use serde_json::{Map, Value};
use url::Url;

/// An answer: its status, headers, and body.
pub(crate) struct Answer {
    pub(crate) status: u16,
    pub(crate) headers: HeaderMap,
    pub(crate) body: Bytes,
}

impl Answer {
    /// The body as a JSON object.
    pub(crate) fn object(&self) -> Option<Map<String, Value>> {
        match serde_json::from_slice(&self.body).ok()? {
            Value::Object(object) => Some(object),
            _ => None,
        }
    }

    pub(crate) fn success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// Why a request got no answer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Trouble {
    /// The outbound policy refused the URL.
    Blocked(Blocked),
    Timeout,
    Unreachable,
    /// The answer passed `OAUTH_RESPONSE_MAX_BYTES`.
    TooLarge,
    Other,
}

impl Trouble {
    pub(crate) fn kind(self) -> &'static str {
        match self {
            Self::Blocked(_) => "blocked",
            Self::Timeout => "timeout",
            Self::Unreachable => "unreachable",
            Self::TooLarge => "too-large",
            Self::Other => "other",
        }
    }
}

/// The clients of one account's OAuth requests.
pub(crate) struct Http<'a> {
    pub(crate) clients: &'a Outbound<Client>,
    pub(crate) network: NetworkScope,
}

impl Http<'_> {
    /// Whether this account may send requests to `url` at all (scheme, credentials in the URL,
    /// an IP-literal host); the clients check every resolved address again.
    pub(crate) fn allows(&self, url: &Url) -> Result<(), Blocked> {
        self.clients.pick(url, self.network).map(|_| ())
    }

    pub(crate) async fn get(&self, url: &Url) -> Result<Answer, Trouble> {
        self.send(url, |client| {
            client.get(url.clone()).header(ACCEPT, "application/json")
        })
        .await
    }

    /// Sends the request `build` makes on the client for `url`, within `OAUTH_TIMEOUT_SECS`.
    pub(crate) async fn send(
        &self,
        url: &Url,
        build: impl FnOnce(&Client) -> RequestBuilder,
    ) -> Result<Answer, Trouble> {
        let client = self
            .clients
            .pick(url, self.network)
            .map_err(Trouble::Blocked)?;
        let request = build(client);
        let answer = async {
            let response = request.send().await.map_err(|error| classify(&error))?;
            let status = response.status().as_u16();
            let headers = response.headers().clone();
            let declared = response.content_length();
            let body = read_capped(declared, response.bytes_stream(), OAUTH_RESPONSE_MAX_BYTES)
                .await
                .map_err(|error| match error {
                    copilot_core::CapError::TooLarge { .. } => Trouble::TooLarge,
                    copilot_core::CapError::Stream(error) => classify(&error),
                })?;
            Ok(Answer {
                status,
                headers,
                body,
            })
        };
        tokio::time::timeout(Duration::from_secs(OAUTH_TIMEOUT_SECS), answer)
            .await
            .map_err(|_| Trouble::Timeout)?
    }
}

fn classify(error: &reqwest::Error) -> Trouble {
    if let Some(blocked) = Blocked::find(error) {
        Trouble::Blocked(blocked)
    } else if error.is_timeout() {
        Trouble::Timeout
    } else if error.is_connect() {
        Trouble::Unreachable
    } else {
        Trouble::Other
    }
}
