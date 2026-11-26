//! The peer address check on every new outbound connection.

use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};

use copilot_core::{Blocked, BoxError};
use hyper_util::client::legacy::connect::{Connection, HttpInfo};
use tower::{Layer, Service};

use crate::outbound::policy::Policy;

/// A reqwest connector layer that checks the peer address of each new connection against the
/// client's [`Policy`] and drops the connection before any request bytes are written.
///
/// hyper skips the resolver for IP-literal hosts (`https://169.254.169.254/`), so this is where
/// the client itself enforces them. `Outbound::pick` (core) already refuses a disallowed literal
/// before a request is built; this layer also covers a request that skips that call, at the
/// cost of a TCP (and TLS) handshake with the refused peer. A connection whose peer is unknown
/// fails closed.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PeerCheckLayer {
    policy: Policy,
}

impl PeerCheckLayer {
    pub(crate) fn new(policy: Policy) -> Self {
        Self { policy }
    }
}

impl<S> Layer<S> for PeerCheckLayer {
    type Service = PeerCheck<S>;

    fn layer(&self, inner: S) -> Self::Service {
        PeerCheck {
            inner,
            policy: self.policy,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PeerCheck<S> {
    inner: S,
    policy: Policy,
}

type Connecting<T> = Pin<Box<dyn Future<Output = Result<T, BoxError>> + Send>>;

impl<S, Request> Service<Request> for PeerCheck<S>
where
    S: Service<Request, Error = BoxError>,
    S::Response: Connection + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = S::Response;
    type Error = BoxError;
    type Future = Connecting<S::Response>;

    fn poll_ready(&mut self, context: &mut Context<'_>) -> Poll<Result<(), BoxError>> {
        self.inner.poll_ready(context)
    }

    fn call(&mut self, request: Request) -> Self::Future {
        let policy = self.policy;
        let connecting = self.inner.call(request);
        Box::pin(async move {
            let connection = connecting.await?;
            match peer(&connection) {
                Some(address) if policy.allows(address.ip()) => Ok(connection),
                _ => {
                    tracing::debug!("security.outbound.peer.blocked");
                    Err(Box::new(Blocked::Address) as BoxError)
                }
            }
        })
    }
}

/// The remote address hyper-util records on a TCP connection (also under TLS).
fn peer(connection: &impl Connection) -> Option<SocketAddr> {
    let mut extensions = http::Extensions::new();
    connection.connected().get_extras(&mut extensions);
    extensions.get::<HttpInfo>().map(HttpInfo::remote_addr)
}
