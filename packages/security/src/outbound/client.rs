//! The hardened outbound `reqwest::Client`s.

use std::time::Duration;

use copilot_core::shared::constants::{
    MCP_IDLE_TIMEOUT_SECS, OUTBOUND_CONNECT_TIMEOUT_SECS, PROVIDER_IDLE_TIMEOUT_SECS,
};
use copilot_core::{NetworkScope, Outbound};
use reqwest::{Client, ClientBuilder, redirect};

use crate::crypto;
use crate::outbound::peer::PeerCheckLayer;
use crate::outbound::policy::Policy;
use crate::outbound::resolver::Resolver;

/// The adapter a set of clients serves. Each adapter gets its own [`Outbound`], because the two
/// differ in idle timeout and connection reuse.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Purpose {
    /// LLM providers: idle timeout `PROVIDER_IDLE_TIMEOUT_SECS`, pooled connections.
    Providers,
    /// MCP servers: idle timeout `MCP_IDLE_TIMEOUT_SECS`, no idle connections kept
    /// (`pool_max_idle_per_host(0)`, as rmcp's own client does to avoid delayed-ACK stalls).
    Tools,
}

/// How long a client waits, and whether it keeps idle connections.
#[derive(Clone, Copy, Debug)]
struct Tuning {
    idle: Duration,
    keep_idle: bool,
}

impl Purpose {
    fn tuning(self) -> Tuning {
        match self {
            Self::Providers => Tuning {
                idle: Duration::from_secs(PROVIDER_IDLE_TIMEOUT_SECS),
                keep_idle: true,
            },
            Self::Tools => Tuning {
                idle: Duration::from_secs(MCP_IDLE_TIMEOUT_SECS),
                keep_idle: false,
            },
        }
    }
}

/// Builds the three hardened clients of one adapter (`docs/design/copilot.md`, Outbound
/// Requests), installing the ring crypto provider first if needed. Every client:
///
/// - resolves names through a resolver that keeps only the addresses its policy allows, and
///   checks the peer of every new connection, because IP-literal hosts skip the resolver;
/// - follows no redirects and ignores proxy settings (a proxy would resolve the target itself);
/// - connects within `OUTBOUND_CONNECT_TIMEOUT_SECS`, ends a response that stays silent past the
///   idle timeout, and has no total timeout, which would cut long streams.
///
/// The public-only and private-allowed clients take `https` only; the plain `http` client reaches
/// only private and loopback addresses. The composition root calls this once per adapter and
/// injects the result; adapters pick a client with [`Outbound::pick`] and cap responses with
/// `copilot_core::read_capped` and `copilot_core::cap_events`.
pub fn clients(purpose: Purpose) -> reqwest::Result<Outbound<Client>> {
    crypto::install();
    let tuning = purpose.tuning();
    Ok(Outbound::new(
        build(Policy::Scope(NetworkScope::Public), tuning)?,
        build(Policy::Scope(NetworkScope::Private), tuning)?,
        build(Policy::Plaintext, tuning)?,
    ))
}

fn build(policy: Policy, tuning: Tuning) -> reqwest::Result<Client> {
    builder(policy, tuning)
        .https_only(policy.https_only())
        .build()
}

/// Everything but the scheme rule, so tests can point a policy at a plain `http` mock server.
fn builder(policy: Policy, tuning: Tuning) -> ClientBuilder {
    let builder = Client::builder()
        .dns_resolver(Resolver::new(policy))
        .connector_layer(PeerCheckLayer::new(policy))
        .redirect(redirect::Policy::none())
        .no_proxy()
        .connect_timeout(Duration::from_secs(OUTBOUND_CONNECT_TIMEOUT_SECS))
        .read_timeout(tuning.idle)
        .user_agent("copilot");
    if tuning.keep_idle {
        builder
    } else {
        builder.pool_max_idle_per_host(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use copilot_core::{Blocked, CapError, cap_events, read_capped};
    use futures_util::StreamExt;
    use reqwest::Url;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn outbound() -> Outbound<Client> {
        clients(Purpose::Providers).expect("the clients build")
    }

    fn url(text: &str) -> Url {
        Url::parse(text).expect("a valid test URL")
    }

    /// A client with `policy` that also takes plain `http`, to reach the mock server.
    fn plain(policy: Policy) -> Client {
        crypto::install();
        builder(policy, Purpose::Tools.tuning())
            .build()
            .expect("the client builds")
    }

    #[tokio::test]
    async fn the_plaintext_client_reaches_private_hosts() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .expect(2)
            .mount(&server)
            .await;
        let outbound = outbound();
        let port = server.address().port();
        for text in [
            format!("http://127.0.0.1:{port}/v1/models"),
            format!("http://localhost:{port}/v1/models"),
        ] {
            let target = url(&text);
            let client = outbound.pick(&target, NetworkScope::Private).unwrap();
            let response = client.get(target).send().await.unwrap();
            assert_eq!(response.status(), 200, "{text}");
        }
    }

    #[tokio::test]
    async fn never_follows_redirects() {
        let server = MockServer::start().await;
        let target = format!("{}/target", server.uri());
        Mock::given(path("/target"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        let outbound = outbound();
        for status in [301_u16, 302, 303, 307, 308] {
            Mock::given(path(format!("/start/{status}")))
                .respond_with(
                    ResponseTemplate::new(status).insert_header("location", target.as_str()),
                )
                .expect(1)
                .mount(&server)
                .await;
            let start = url(&format!("{}/start/{status}", server.uri()));
            let client = outbound.pick(&start, NetworkScope::Private).unwrap();
            let response = client.post(start).body("{}").send().await.unwrap();
            assert_eq!(response.status(), status, "the redirect comes back as is");
        }
    }

    #[tokio::test]
    async fn checks_the_peer_of_ip_literal_hosts() {
        let server = MockServer::start().await;
        let client = plain(Policy::Scope(NetworkScope::Public));
        let error = client
            .get(format!("{}/", server.uri()))
            .send()
            .await
            .expect_err("a public-only client must not reach 127.0.0.1");
        assert!(error.is_connect(), "{error:?}");
        assert_eq!(Blocked::find(&error), Some(Blocked::Address));
        let received = server.received_requests().await.unwrap_or_default();
        assert!(received.is_empty(), "no request bytes reached the server");
    }

    #[tokio::test]
    async fn the_resolver_drops_disallowed_addresses() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let client = plain(Policy::Scope(NetworkScope::Public));
        let error = client
            .get(format!("http://localhost:{port}/"))
            .send()
            .await
            .expect_err("localhost resolves to loopback only");
        assert!(error.is_connect(), "{error:?}");
        assert_eq!(Blocked::find(&error), Some(Blocked::Address));
        // Refused at resolution: no connection was even opened (the peer check would have
        // opened one and dropped it).
        let accepted = tokio::time::timeout(Duration::from_millis(200), listener.accept()).await;
        assert!(accepted.is_err(), "no connection reached the listener");
    }

    #[tokio::test]
    async fn https_clients_refuse_plain_http() {
        let server = MockServer::start().await;
        let outbound = outbound();
        for scope in [NetworkScope::Public, NetworkScope::Private] {
            let client = outbound
                .pick(&url("https://api.example.com/"), scope)
                .unwrap();
            let error = client
                .get(format!("{}/", server.uri()))
                .send()
                .await
                .expect_err("an https client never sends plain http");
            assert!(error.is_builder(), "{error:?}");
        }
        let received = server.received_requests().await.unwrap_or_default();
        assert!(received.is_empty());
    }

    #[tokio::test]
    async fn ends_a_silent_response_at_the_idle_timeout() {
        let server = MockServer::start().await;
        Mock::given(path("/slow"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(2)))
            .mount(&server)
            .await;
        crypto::install();
        let tuning = Tuning {
            idle: Duration::from_millis(200),
            keep_idle: false,
        };
        let client = builder(Policy::Plaintext, tuning).build().unwrap();
        let error = client
            .get(format!("{}/slow", server.uri()))
            .send()
            .await
            .expect_err("the response is silent past the idle timeout");
        assert!(error.is_timeout(), "{error:?}");
    }

    #[tokio::test]
    async fn caps_responses_read_through_core() {
        let server = MockServer::start().await;
        Mock::given(path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_string("0123456789"))
            .mount(&server)
            .await;
        Mock::given(path("/stream"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string("data: ok\n\ndata: far too long\n\n"),
            )
            .mount(&server)
            .await;
        let outbound = outbound();
        let models = url(&format!("{}/models", server.uri()));
        let client = outbound.pick(&models, NetworkScope::Private).unwrap();

        let response = client.get(models.clone()).send().await.unwrap();
        let declared = response.content_length();
        let body = read_capped(declared, response.bytes_stream(), 10)
            .await
            .unwrap();
        assert_eq!(&body[..], b"0123456789");
        let response = client.get(models).send().await.unwrap();
        let declared = response.content_length();
        let error = read_capped(declared, response.bytes_stream(), 9)
            .await
            .unwrap_err();
        assert!(matches!(error, CapError::TooLarge { limit: 9 }));

        let response = client
            .get(format!("{}/stream", server.uri()))
            .send()
            .await
            .unwrap();
        let results: Vec<_> = cap_events(response.bytes_stream(), 10).collect().await;
        assert!(
            results
                .iter()
                .any(|result| matches!(result, Err(CapError::TooLarge { limit: 10 })))
        );
    }

    #[test]
    fn tunes_each_purpose() {
        let providers = Purpose::Providers.tuning();
        assert_eq!(providers.idle, Duration::from_secs(300));
        assert!(providers.keep_idle);
        let tools = Purpose::Tools.tuning();
        assert_eq!(tools.idle, Duration::from_secs(60));
        assert!(!tools.keep_idle);
    }
}
