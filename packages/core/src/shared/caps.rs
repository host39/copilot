//! Response size caps of the outbound adapters (`docs/design/copilot.md`, Outbound Requests).
//!
//! reqwest has no body limit of its own, so the provider adapter reads responses through these:
//! [`read_capped`] for whole bodies (model lists, error bodies) and [`cap_events`] for SSE
//! streams, where the cap applies to each event and never to the whole stream (a long answer is
//! many events). The MCP adapter passes the same cap to rmcp (`max_json_response_size`,
//! `max_sse_event_size`). They work on any byte stream (`reqwest::Response::bytes_stream`), so
//! the adapters need no dependency beyond core.

use std::pin::pin;

use bytes::{Bytes, BytesMut};
use futures_util::{Stream, StreamExt};

/// A response that broke a size cap, or the stream error that ended it.
#[derive(Debug, thiserror::Error)]
pub enum CapError<E> {
    #[error("outbound.response.too-large {limit}")]
    TooLarge { limit: usize },
    #[error("outbound.response.failed")]
    Stream(#[source] E),
}

/// Reads a whole body of at most `limit` bytes. A larger `declared` length (`Content-Length`)
/// fails before reading; a body without one fails as soon as it passes the limit.
pub async fn read_capped<S, E>(
    declared: Option<u64>,
    stream: S,
    limit: usize,
) -> Result<Bytes, CapError<E>>
where
    S: Stream<Item = Result<Bytes, E>>,
{
    if declared.is_some_and(|length| length > limit as u64) {
        return Err(CapError::TooLarge { limit });
    }
    let mut stream = pin!(stream);
    let mut body = BytesMut::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(CapError::Stream)?;
        if body.len() + chunk.len() > limit {
            return Err(CapError::TooLarge { limit });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body.freeze())
}

/// Passes an SSE byte stream through unchanged and fails once one event (the bytes between two
/// blank lines, line endings excluded) exceeds `limit` bytes, before an SSE parser buffers more
/// of it. Line endings may be LF, CRLF, or CR, as the SSE format allows. Every item after the
/// first failure fails too.
pub fn cap_events<S, E>(stream: S, limit: usize) -> impl Stream<Item = Result<Bytes, CapError<E>>>
where
    S: Stream<Item = Result<Bytes, E>>,
{
    let mut sizer = EventSizer::new(limit);
    stream.map(move |chunk| {
        let chunk = chunk.map_err(CapError::Stream)?;
        sizer.feed(&chunk)?;
        Ok(chunk)
    })
}

/// Tracks the size of the SSE event in progress across chunks.
#[derive(Debug)]
struct EventSizer {
    limit: usize,
    /// Bytes of the current event, line endings excluded.
    event: usize,
    /// Bytes of the current line.
    line: usize,
    /// The last byte was a CR, so a following LF belongs to the same line ending.
    after_cr: bool,
    over: bool,
}

impl EventSizer {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            event: 0,
            line: 0,
            after_cr: false,
            over: false,
        }
    }

    fn feed<E>(&mut self, chunk: &[u8]) -> Result<(), CapError<E>> {
        let too_large = CapError::TooLarge { limit: self.limit };
        if self.over {
            return Err(too_large);
        }
        for &byte in chunk {
            match byte {
                b'\n' if self.after_cr => self.after_cr = false,
                b'\r' | b'\n' => {
                    self.after_cr = byte == b'\r';
                    if self.line == 0 {
                        self.event = 0;
                    }
                    self.line = 0;
                }
                _ => {
                    self.after_cr = false;
                    self.line += 1;
                    self.event += 1;
                    if self.event > self.limit {
                        self.over = true;
                        return Err(too_large);
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Chunk = Result<Bytes, std::io::Error>;

    fn sizes(limit: usize, chunks: &[&[u8]]) -> Result<(), CapError<()>> {
        let mut sizer = EventSizer::new(limit);
        chunks.iter().try_for_each(|chunk| sizer.feed(chunk))
    }

    fn stream(chunks: &[&'static [u8]]) -> impl Stream<Item = Chunk> {
        futures_util::stream::iter(
            chunks
                .iter()
                .map(|chunk| Ok(Bytes::from_static(chunk)))
                .collect::<Vec<Chunk>>(),
        )
    }

    #[test]
    fn allows_many_events_within_the_limit() {
        assert!(sizes(11, &[&b"data: 12345\n\n".repeat(1_000)]).is_ok());
        assert!(sizes(11, &[&b"data: 12345\r\n\r\n".repeat(100)]).is_ok());
        assert!(sizes(11, &[&b"data: 12345\r\r".repeat(100)]).is_ok());
    }

    #[test]
    fn counts_every_line_of_an_event() {
        // Two 6-byte lines make a 12-byte event.
        assert!(sizes(12, &[b"data: \ndata: \n\n"]).is_ok());
        assert!(matches!(
            sizes(11, &[b"data: \ndata: \n\n"]),
            Err(CapError::TooLarge { limit: 11 })
        ));
        assert!(sizes(11, &[b"data: \r\ndata: \r\n\r\n"]).is_err());
    }

    #[test]
    fn fails_an_event_split_across_chunks() {
        assert!(sizes(8, &[b"data: ", b"abc", b"\n\n"]).is_err());
        assert!(sizes(9, &[b"data: ", b"abc", b"\n\n", b"data: ", b"abc"]).is_ok());
    }

    #[test]
    fn a_crlf_split_across_chunks_is_one_line_ending() {
        // "\r" then "\n" in the next chunk is one line ending, not a blank line, so the event
        // continues and its second line still counts.
        assert!(sizes(5, &[b"abc\r", b"\ndef\n\n"]).is_err());
        assert!(sizes(6, &[b"abc\r", b"\ndef\n\n"]).is_ok());
    }

    #[tokio::test]
    async fn the_stream_fails_at_the_oversized_event_and_after() {
        let chunks = stream(&[b"data: ok\n\n", b"data: far too long\n\n", b"data: ok\n\n"]);
        let results: Vec<_> = cap_events(chunks, 10).collect().await;
        assert!(results[0].is_ok());
        assert!(matches!(results[1], Err(CapError::TooLarge { limit: 10 })));
        assert!(results[2].is_err());
    }

    #[tokio::test]
    async fn passes_stream_errors_through() {
        let failing =
            futures_util::stream::iter(vec![Err::<Bytes, _>(std::io::Error::other("reset"))]);
        let results: Vec<_> = cap_events(failing, 10).collect().await;
        assert!(matches!(results[0], Err(CapError::Stream(_))));
    }

    #[tokio::test]
    async fn reads_bodies_within_the_limit() {
        let body = read_capped(Some(10), stream(&[b"0123456789"]), 10)
            .await
            .unwrap();
        assert_eq!(&body[..], b"0123456789");
        let body = read_capped(None, stream(&[b"01234", b"56789"]), 10)
            .await
            .unwrap();
        assert_eq!(&body[..], b"0123456789");
    }

    #[tokio::test]
    async fn refuses_a_declared_length_over_the_limit_before_reading() {
        let unread = futures_util::stream::poll_fn(|_| -> std::task::Poll<Option<Chunk>> {
            panic!("a body over its declared limit is never read")
        });
        let error = read_capped(Some(10), unread, 9).await.unwrap_err();
        assert!(matches!(error, CapError::TooLarge { limit: 9 }));
        assert_eq!(error.to_string(), "outbound.response.too-large 9");
    }

    #[tokio::test]
    async fn stops_reading_an_undeclared_body_at_the_limit() {
        let error = read_capped(None, stream(&[b"01234", b"56789", b"more"]), 9)
            .await
            .unwrap_err();
        assert!(matches!(error, CapError::TooLarge { limit: 9 }));
    }
}
