# Dependencies Research

Status: research note, checked against live primary sources (crates.io API, npm registry, docs.rs, crate sources, MDN browser-compat-data) on 2026-11-26. Treat it as a draft: versions move weekly, so re-check the linked pages before pinning. Claims I could not confirm from a primary source are marked **UNVERIFIED**. Sizes and crate counts marked **measured** come from throwaway builds described in [Verification Method](#verification-method).

Scope: the third-party crates and npm packages Copilot needs for provider forwarding, streaming, uploads, secrets, passwords, schedules, and the chat UI, chosen for a small footprint. MCP (`rmcp`) is covered in [mcp.md](./mcp.md), provider wire formats in [providers.md](./providers.md), and audio in [voice.md](./voice.md).

## Key Findings

1. **reqwest 0.13 defaults to the aws-lc-rs crypto provider, but our sqlx uses ring.** Taking reqwest's default `rustls` feature would compile both providers. Use `rustls-no-provider` and install ring once at startup. Without that call, reqwest panics when building a `Client`. Sources: [reqwest CHANGELOG v0.13.0](https://github.com/seanmonstar/reqwest/blob/master/CHANGELOG.md), [reqwest client.rs v0.13.5](https://github.com/seanmonstar/reqwest/blob/v0.13.5/src/async_impl/client.rs) (`default_rustls_crypto_provider`), [sqlx-core 0.9.1 tls_rustls.rs](https://docs.rs/crate/sqlx-core/0.9.1/source/src/net/tls/tls_rustls.rs).
2. **sqlx 0.9.1 already exposes `chrono`, `json`, and `uuid`** (exact feature names), and `packages/database/Cargo.toml` already enables all three. Nothing to add for the database layer. Source: [crates.io sqlx](https://crates.io/api/v1/crates/sqlx).
3. **chrono-tz 0.10.4 (latest release) bundles tzdata 2025b; IANA is at 2026f.** Seven tzdb releases are missing, including future-time changes such as Cuba not falling back on 2026-11-01. jiff 1.0.3 reads the system `/usr/share/zoneinfo` first and can fall back to `jiff-tzdb` 1.0.0, which bundles 2026f. Recommendation: `croner` (cron) on the `jiff` backend for next-fire computation, chrono stays for sqlx and contracts. Sources: [chrono-tz 0.10.4 tz/NEWS](https://docs.rs/crate/chrono-tz/0.10.4/source/tz/NEWS), [IANA NEWS](https://data.iana.org/time-zones/tzdb/NEWS), [jiff-tzdb 1.0.0 tzname.rs](https://docs.rs/crate/jiff-tzdb/1.0.0/source/tzname.rs), [jiff TimeZoneDatabase](https://docs.rs/jiff/1.0.3/jiff/tz/struct.TimeZoneDatabase.html).
4. **Use `croner` 5.0.1 for cron.** It parses 5-field patterns, supports `L`, `W`, `#`, nicknames such as `@daily`, and documents DST gap and overlap rules. `cron` 0.17.0 needs a seconds field and lacks `L`/`W`; `saffron` 0.1.0 has not been released since 2021-02-01. Sources: [croner README](https://github.com/hexagon/croner-rust/blob/main/README.md), [cron docs](https://docs.rs/cron/0.17.0/cron/), [crates.io saffron](https://crates.io/api/v1/crates/saffron).
5. **SSE parsing: `sse-stream` 0.3.0**, not `eventsource-stream` (last release 2022-02-17, pulls `nom` 7) or `reqwest-eventsource` (requires reqwest ^0.12, so it would add a second reqwest). Sources: [crates.io sse-stream](https://crates.io/api/v1/crates/sse-stream), [crates.io eventsource-stream](https://crates.io/api/v1/crates/eventsource-stream), [reqwest-eventsource 0.6.0 dependencies](https://crates.io/api/v1/crates/reqwest-eventsource/0.6.0/dependencies).
6. **Argon2: keep the `argon2` 0.6.0 defaults.** `Argon2::default()` is Argon2id v0x13 (19) with `Params::DEFAULT`: m = 19 MiB (19456 KiB), t = 2, p = 1 (`Params` itself carries only the costs; the algorithm and version come from `Algorithm::default()` and `Version::default()`), which is exactly one of OWASP's minimum configurations. Sources: [argon2 0.6.0 params.rs](https://docs.rs/crate/argon2/0.6.0/source/src/params.rs), [OWASP Password Storage Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html).
7. **Measured footprint:** the proposed Rust lines add 42 crates to the 171-crate normal dependency graph (213 total, about 25% more), because sqlx already brings rustls, ring, sha2, chrono, uuid, rand, getrandom, and tokio-stream. `wiremock` adds 14 more, dev-only.
8. **Web: 4 runtime packages are worth it:** `react-markdown` + `remark-gfm` (50.4 KB gzip measured), `@tanstack/react-query` (10.5 KB), and optionally `eventsource-parser` (2.0 KB). Skip a syntax highlighter in v1 or lazy-load `highlight.js` core per language. Do not use `@microsoft/fetch-event-source` (last published 2021-04-25, and by default it closes and **re-opens** the request when the tab is hidden and shown again, which would re-POST a chat message). Sources: [npm registry](https://registry.npmjs.org/@microsoft/fetch-event-source), [fetch-event-source README](https://github.com/Azure/fetch-event-source/blob/main/README.md).
9. **Do not iterate `ReadableStream` with `for await` in the web client.** Async iteration landed in Chrome 124, Firefox 110, and Safari 27, so older WebKit (WebKitGTK on the Linux client) may lack it. Use `reader.read()` in a loop. Source: [BCD ReadableStream.json](https://github.com/mdn/browser-compat-data/blob/main/api/ReadableStream.json).

## Workspace Baseline

From `/Cargo.toml`, `Cargo.lock`, and the crate manifests on 2026-11-26:

| Item | Declared | Resolved in `Cargo.lock` |
| --- | --- | --- |
| Toolchain | `rust-toolchain.toml` channel `stable`; workspace `rust-version = "1.94"` | rustc 1.97.0 in this environment |
| axum | `0.8.9` | 0.8.9 (axum-core 0.5.6) |
| tokio | `1.48.0` | 1.54.1 |
| sqlx | `0.9.1`, features `chrono`, `json`, `macros`, `migrate`, `postgres`, `runtime-tokio`, `tls-rustls-ring-webpki`, `uuid` | 0.9.1 (MSRV 1.94.0, which is why the workspace says 1.94) |
| tower-http | `0.7.1` (middleware uses `cors`) | 0.7.1 |
| uuid | `1.18.1` (middleware adds `v4`) | 1.29.3 |

Already in the lock file through sqlx and axum, so adding them costs no new crates: `rustls` 0.23.46, `ring` 0.17.14, `sha2` 0.11.2, `chrono` 0.4.45, `uuid` 1.29.3, `rand` 0.10.4, `getrandom` 0.4.3, `tokio-stream` 0.1.19, `futures-core`/`futures-util` 0.3.34, `bytes` 1.12.2, `base64` 0.22.1, `webpki-roots` 1.0.12 (from `cargo tree --offline -i <crate>`).

Postgres is `postgres:18.1` (`docker-compose.yml`), which has a native `uuidv7()` function. Source: [PostgreSQL 18 UUID functions](https://www.postgresql.org/docs/18/functions-uuid.html).

## Rust Crates

### Summary

Versions from the crates.io API (`https://crates.io/api/v1/crates/<name>`) on 2026-11-26. All of them compiled together with edition 2024 on rustc 1.97.0 (measured), and every declared MSRV is at or below the workspace's 1.94.

| Crate | Latest stable | Released | MSRV | Decision |
| --- | --- | --- | --- | --- |
| `reqwest` | 0.13.5 | 2026-09-08 | 1.85 | **Use**, no default features |
| `rustls` (direct) | 0.23.46 | 2026-10-14 | 1.71 | **Use** only to install the ring provider |
| `sse-stream` | 0.3.0 | 2026-09-18 | none declared | **Use** |
| `eventsource-stream` | 0.2.3 | 2022-02-17 | none | Reject (stale, `nom` 7) |
| `reqwest-eventsource` | 0.6.0 | 2024-03-29 | none | Reject (reqwest ^0.12) |
| `futures-core` | 0.3.34 | 2026-08-11 | 1.71 (futures family) | Not used (P6.1: `futures-util` covers the port types; removed from the workspace) |
| `futures-util` | 0.3.34 | 2026-08-11 | 1.71 | **Use** in adapters |
| `futures` (facade) | 0.3.34 | 2026-08-11 | 1.71 | Not needed directly |
| `tokio-stream` | 0.1.19 | 2026-07-22 | 1.71 | Not used (P6.1: the run registry streams through `futures-util`; removed from the workspace) |
| `tokio-util` | 0.7.20 | 2026-10-09 | 1.85 | **Use** (`CancellationToken`) |
| `bytes` | 1.12.2 | 2026-11-17 | 1.57 | **Use** |
| `async-stream` | 0.3.6 | 2024-10-01 | 1.65 | Not needed |
| `chrono` | 0.4.45 | 2026-06-04 | 1.62.0 | **Use** (sqlx, contracts) |
| `time` | 0.3.55 | 2026-08-01 | 1.88.0 | Not needed |
| `jiff` | 1.0.3 | 2026-11-24 | 1.85 | **Use** for time zones and cron |
| `chrono-tz` | 0.10.4 | 2025-07-11 | 1.65 | Reject (tzdata 2025b) |
| `croner` | 5.0.1 | 2026-11-25 | 1.85 | **Use** |
| `cron` | 0.17.0 | 2026-06-18 | none | Reject |
| `saffron` | 0.1.0 | 2021-02-01 | none | Reject |
| `uuid` | 1.29.3 | 2026-11-17 | 1.89.0 | **Use** (bump the workspace line) |
| `aes-gcm` | 0.11.1 | 2026-08-21 | 1.85 | **Use** |
| `chacha20poly1305` | 0.11.0 | 2026-06-28 | 1.85 | Alternative, not needed |
| `getrandom` | 0.4.3 | 2026-06-17 | 1.85 | **Use** |
| `rand` | 0.10.4 | 2026-11-15 | 1.85 | Not needed directly |
| `base64` | 0.23.1 | 2026-08-04 | 1.71.0 | **Use** |
| `argon2` | 0.6.0 | 2026-08-27 | 1.85 | **Use** |
| `sha2` | 0.11.2 | 2026-11-25 | 1.88 | **Use** |
| `mime_guess` | 2.0.5 | 2024-06-29 | none | Not used (P6.1: core detects types from the extension and magic bytes; removed from the workspace) |
| `wiremock` (dev) | 0.6.5 | 2025-08-24 | none (edition 2024) | **Use** |
| `tower-http` | 0.7.1 | 2026-08-31 | 1.65 | Add the `limit` feature |
| `axum` | 0.8.9 | 2026-04-14 | 1.80 | Add the `multipart` feature where uploads live |
| `miniz_oxide` | 0.9.1 | 2026-03-13 | none declared | **Use** in `copilot-core` (added in the P1.3 review): inflates PDF object streams so `pdf_pages` counts pages that PDF 1.5+ files compress; pure Rust, `#![forbid(unsafe_code)]`, brings only `adler2` |

### Proposed Workspace Lines

Add to `[workspace.dependencies]` in `/Cargo.toml` (crates then use `{ workspace = true }`, per the `rust-imports` rule). These exact lines compiled together and passed a small test (measured):

```toml
aes-gcm = "0.11.1"
argon2 = "0.6.0"
base64 = "0.23.1"
bytes = "1.12.2"
chrono = { version = "0.4.45", default-features = false, features = ["std", "clock", "serde"] }
croner = { version = "5.0.1", default-features = false, features = ["std", "jiff"] }
futures-util = { version = "0.3.34", default-features = false, features = ["std"] }
getrandom = "0.4.3"
jiff = { version = "1.0.3", default-features = false, features = ["std", "tzdb-zoneinfo", "tzdb-bundle-always"] }
reqwest = { version = "0.13.5", default-features = false, features = ["rustls-no-provider", "json", "stream", "multipart"] }
rustls = { version = "0.23.46", default-features = false, features = ["ring", "std", "tls12"] }
sha2 = "0.11.2"
sse-stream = { version = "0.3.0", default-features = false }
tokio-util = { version = "0.7.20", default-features = false }
wiremock = "0.6.5"
```

Change existing lines:

```toml
axum = "0.8.9"                       # unchanged; crates that accept uploads use: axum = { workspace = true, features = ["multipart"] }
tower-http = "0.7.1"                 # unchanged; middleware uses: tower-http = { workspace = true, features = ["cors", "limit"] }
uuid = { version = "1.29.3", features = ["v4", "v7", "serde"] }   # was "1.18.1"
```

Suggested placement (matches the crate plan in `docs/design/copilot.md`):

| Crate | New dependencies |
| --- | --- |
| `copilot-core` | `chrono`, `uuid`, `croner`, `jiff`, `miniz_oxide`, `idna_adapter` 1.1 (pinned: the unicode-rs back end of `url`'s IDNA mapping instead of ICU4X, about twenty crates fewer) (all pure libraries, allowed by the design's boundary note) |
| `copilot-providers` | `reqwest`, `sse-stream`, `futures-util`, `bytes`, `base64`, `tokio-util`; dev: `wiremock`, `tokio` |
| `copilot-security` | `aes-gcm`, `argon2`, `sha2`, `getrandom`, `base64` |
| `copilot-storage` | `sha2`, `bytes`, `tokio-util` (add its `io` feature only if streaming files with `ReaderStream`) |
| `copilot-api` | `axum` `multipart`, `rustls` (provider install), `hyper` and `hyper-util` (the HTTP/1 server with a header read timeout, already in the tree through axum) |
| `copilot-middleware` | `tower-http` `limit` |
| `copilot-database` | none (sqlx features already present) |

### reqwest and TLS

- Latest 0.13.5 (2026-09-08), MSRV 1.85. Source: [crates.io reqwest](https://crates.io/api/v1/crates/reqwest).
- 0.13.0 breaking changes: rustls is the default backend, "`rustls` crypto provider defaults to aws-lc instead of _ring_" (`rustls-no-provider` exists for other providers), `rustls-tls` was renamed `rustls`, root-store features were removed in favour of `rustls-platform-verifier`, and `query`/`form` became opt-in. Source: [CHANGELOG](https://github.com/seanmonstar/reqwest/blob/master/CHANGELOG.md).
- Default features are `default-tls` (= `rustls` = aws-lc-rs + platform verifier), `charset`, `http2`, `system-proxy`. Source: [crates.io reqwest features](https://crates.io/api/v1/crates/reqwest), [reqwest Cargo.toml v0.13.5](https://github.com/seanmonstar/reqwest/blob/v0.13.5/Cargo.toml).
- With `rustls-no-provider`, reqwest uses the process default provider and otherwise panics: "No rustls crypto provider is configured. When using the `rustls-no-provider` feature you must install a crypto provider before building a Client." Source: [client.rs v0.13.5](https://github.com/seanmonstar/reqwest/blob/v0.13.5/src/async_impl/client.rs).
- sqlx 0.9.1 with `tls-rustls-ring-webpki` builds its own config with `rustls::crypto::ring::default_provider()` via `builder_with_provider`, so a ring process default does not change sqlx. Source: [sqlx-core tls_rustls.rs](https://docs.rs/crate/sqlx-core/0.9.1/source/src/net/tls/tls_rustls.rs). One exception: with `accept_invalid_hostnames`, sqlx builds the inner verifier with `WebPkiServerVerifier::builder`, which takes the process default (`CryptoProvider::get_default_or_install_from_crate_features()` in [rustls 0.23.46 server_verifier.rs](https://docs.rs/crate/rustls/0.23.46/source/src/webpki/server_verifier.rs)); with ring installed that is still ring.
- Install once in each binary's `main` (server, lambda) before any `reqwest::Client` is built, and with `let _ =` in tests (it returns `Err` if already installed):

```rust
rustls::crypto::ring::default_provider()
    .install_default()
    .expect("rustls crypto provider is installed once at startup");
```

- Features to drop and why:
  - `http2`: providers and MCP servers all serve HTTP/1.1 and SSE streams fine over it; dropping it removed `h2` and `fnv` (measured). Turn it back on only if a provider needs it.
  - `charset`: only affects `Response::text()` for non-UTF-8 bodies; provider JSON is UTF-8.
  - `system-proxy`: reads Windows and macOS proxy settings only. Environment proxies (`HTTPS_PROXY`, `NO_PROXY`, ...) are still honoured, because hyper-util's `Matcher::from_system()` "checks the same environment variables as `from_env()`" and adds OS settings only with that feature. Source: [hyper-util matcher.rs 0.1.21](https://docs.rs/crate/hyper-util/0.1.21/source/src/client/proxy/matcher.rs).
- Keep `json`, `stream` (needed for `Response::bytes_stream()`), and `multipart` (transcription uploads, see voice.md).
- Root certificates: the platform verifier on Linux reads the system store (`rustls-native-certs`, `openssl-probe` appear in the tree, measured). `apps/api/Dockerfile` already installs `ca-certificates` in the runtime image. Whether the Lambda `provided.al2023` runtime exposes a CA bundle the verifier finds is **UNVERIFIED**; `ClientBuilder::tls_certs_only(...)` can supply roots explicitly if needed ([client.rs](https://github.com/seanmonstar/reqwest/blob/v0.13.5/src/async_impl/client.rs)).
- Unavoidable duplicates: reqwest 0.13.5 depends on `tower-http` 0.6 (`follow-redirect`) while we use 0.7.1, and reqwest/hyper-util use `base64` 0.23 while sqlx uses 0.22 (measured with `cargo tree -d`). Picking `base64` 0.23.1 for our code therefore costs nothing extra.
- Keep aws-lc-rs out when MCP lands: in `rmcp` 3.12.0 the `reqwest` feature enables `reqwest?/rustls` (aws-lc-rs) and `auth-client-credentials-jwt` pulls `jsonwebtoken` with `aws_lc_rs`; leave both off (mcp.md already recommends passing our own client) or use `reqwest-tls-no-provider`. Source: rmcp 3.12.0 `Cargo.toml` `[features]` ([docs.rs source](https://docs.rs/crate/rmcp/3.12.0/source/Cargo.toml)).
- Alternative: switch sqlx to `tls-rustls-aws-lc-rs` and keep reqwest's default `rustls`, which avoids the install call but builds the aws-lc C library. Compile time and binary size of that path are **UNVERIFIED** here; ring keeps the current build unchanged.

### SSE Parsing

- `sse-stream` 0.3.0 (2026-09-18, MIT OR Apache-2.0, about 24.5 M recent downloads). `SseStream::from_bytes_stream(response.bytes_stream())` yields `Sse { event, data, id, retry }` items (measured against a wiremock SSE body). Source: [crates.io sse-stream](https://crates.io/api/v1/crates/sse-stream).
- 0.3.0 changelog: unknown fields are ignored instead of rejected, repeated `event`/`id`/`retry` keep the last valid value, the encoder normalises CRLF and bare CR in `data` to LF (the decoder accepts LF, CRLF, and bare CR line endings, per the crate's `tests/test_bytes_parse/fields.rs`), decoders emit an error once then end, and parser scratch buffers retain at most 64 KiB, which "bounds retained scratch capacity, not the size of an in-progress event". Source: [sse-stream 0.3.0 CHANGELOG](https://docs.rs/crate/sse-stream/0.3.0/source/CHANGELOG.md). So the provider adapter should still cap per-event and per-response size itself.
- `rmcp` 3.12.0 depends on `sse-stream` ^0.2.4, so with MCP enabled the tree holds both 0.2.x and 0.3.0 (a small crate: about 1,500 lines of Rust in 0.3.0's `src`). Source: [rmcp 3.12.0 dependencies](https://crates.io/api/v1/crates/rmcp/3.12.0/dependencies). Using 0.2.6 to share rmcp's copy is possible, but 0.2.x rejects unknown and duplicated fields, which is less spec-tolerant.
- Default features `memchr` and `simdutf8` are speedups with standard-library fallbacks; `default-features = false` compiled and parsed correctly (measured).
- An in-house parser over `bytes_stream()` (about 100 lines) remains a fine alternative if we want the size caps inside the parser.

### Streams, Cancellation, and Bytes

- `copilot-core` ports can return `futures_core::stream::BoxStream<'static, Result<StreamEvent, DomainError>>`; `BoxStream` is in `futures-core` behind its default `alloc`/`std` feature, so core does not need `futures-util`. Source: [futures-core 0.3.34 stream.rs](https://docs.rs/crate/futures-core/0.3.34/source/src/stream.rs).
- `futures-util` with `default-features = false, features = ["std"]` provides `StreamExt`, `TryStreamExt`, `Stream` (measured).
- `tokio-stream` with no features still provides `wrappers::ReceiverStream` (measured). Pattern for the chat endpoint: the conversation engine runs in a spawned task and writes `Event`s to an `mpsc` channel; the handler returns `Sse::new(ReceiverStream::new(rx).map(Ok))`. This also fits the design's reattach endpoint (`GET /chats/{id}/stream`), because the generation task outlives a dropped HTTP connection.
- `async-stream` is not needed: the channel pattern covers it without a proc macro.
- `tokio-util` with no features provides `sync::CancellationToken` for `POST /chats/{id}/stop` (measured). Add `io` only for `ReaderStream`. reqwest's `stream` feature already pulls tokio-util, so this adds no crate.
- `bytes` 1.12.2 is already resolved through axum and http.

### Dates, IDs, and Schedules

**chrono stays the storage and wire type.** sqlx's `chrono` feature is already on, utoipa 6.0.0 has a `chrono` feature, and ts-rs 12.0.1 has `chrono-impl`. Their jiff support targets jiff 0.2 (`jiff_0_2` in utoipa, `jiff ^0.2` in ts-rs), not jiff 1.0. Sources: [crates.io utoipa](https://crates.io/api/v1/crates/utoipa), [ts-rs 12.0.1 dependencies](https://crates.io/api/v1/crates/ts-rs/12.0.1/dependencies). (`jiff-sqlx` 1.0.0 supports sqlx ^0.9 if we ever move fully to jiff: [crates.io jiff-sqlx](https://crates.io/api/v1/crates/jiff-sqlx).) `time` 0.3.55 has no advantage here.

**jiff for time zones, croner for cron.**

- croner 5.0.1 (2026-11-25, MIT, MSRV 1.85). Releases: 4.0.0 on 2026-08-31, 5.0.0 on 2026-11-21, so pin the exact version and read [MIGRATION.md](https://github.com/hexagon/croner-rust/blob/main/MIGRATION.md) on bumps. Source: [crates.io croner](https://crates.io/api/v1/crates/croner).
- Features: `std` (default), `chrono` (default), `jiff` (optional, jiff 1.0), `serde`. Source: [croner README](https://github.com/hexagon/croner-rust/blob/main/README.md).
- Next fire time in a zone: `cron.find_next_occurrence(&zoned, false)` where `zoned` is a `jiff::ZonedDateTime` (or a chrono `DateTime<Tz>`). Source: same README.
- The default parser accepts optional seconds and year fields (`Seconds::Optional`, `Year::Optional` are the `#[default]` variants in [parser.rs](https://docs.rs/crate/croner/5.0.1/source/src/parser.rs)). Restrict user and model input to 5 fields so nobody can schedule every second:

```rust
use croner::parser::{CronParser, Seconds, Year};

let parser = CronParser::builder()
    .seconds(Seconds::Disallowed)
    .year(Year::Disallowed)
    .build();
let rule = parser.parse("30 9 * * MON-FRI")?;
let zone = jiff::tz::TimeZone::get("America/New_York")?;
let next = rule.find_next_occurrence(&jiff::Timestamp::now().to_zoned_datetime(zone), false)?;
let next_utc = chrono::DateTime::from_timestamp(next.timestamp().as_second(), next.timestamp().subsec_nanosecond() as u32);
```

Measured: with this parser a 6-field pattern is rejected, and `30 2 * * *` in `America/New_York` starting 2026-03-07 fires at 2026-03-08T03:00-04:00 on the spring-forward day, then at 02:30 on the following days, matching the README's fixed-time job rule. jiff 1.0 renamed `Zoned` to `ZonedDateTime`, and the conversion is `Timestamp::to_zoned_datetime`.

- DST: fixed-time jobs run once at the first valid time after a gap and once in an overlap; interval and wildcard jobs skip gap times. Source: [README, DST section](https://github.com/hexagon/croner-rust/blob/main/README.md).
- `cron` 0.17.0 uses a 7-field format with seconds (`"0 30 9,12,15 1,15 May-Aug Mon,Wed,Fri 2018/2"`), and users' familiar 5-field patterns fail (measured: `Schedule::from_str("30 9 * * MON-FRI")` errors, the 6-field form parses). Source: [cron docs](https://docs.rs/cron/0.17.0/cron/). croner's own comparison lists no `L`/`W` in `cron` ([README](https://github.com/hexagon/croner-rust/blob/main/README.md)).
- Time zone data:
  - chrono-tz 0.10.4 is the latest release and its `tz/NEWS` starts at "Release 2025b"; IANA's NEWS lists 2025c and 2026a through 2026f since then. The `chrono-tz` main branch is at 0.11.0, unreleased. Sources: [chrono-tz 0.10.4 NEWS](https://docs.rs/crate/chrono-tz/0.10.4/source/tz/NEWS), [IANA NEWS](https://data.iana.org/time-zones/tzdb/NEWS), [chrono-tz Cargo.toml on main](https://github.com/chronotope/chrono-tz/blob/main/chrono-tz/Cargo.toml).
  - jiff looks up `TZDIR` or `/usr/share/zoneinfo` first, then a concatenated database, then the bundled copy when `tzdb-bundle-always` is on; jiff's docs say the system copy is "heavily preferred". Source: [jiff db/mod.rs](https://docs.rs/crate/jiff/1.0.3/source/src/tz/db/mod.rs).
  - jiff-tzdb 1.0.0 bundles `2026f` (about 200 KB of data). Source: [tzname.rs](https://docs.rs/crate/jiff-tzdb/1.0.0/source/tzname.rs).
  - Measured: this container's system tzdata is 2026e; jiff with the system database computes 2026-11-15 12:00 in `America/Havana` as `-05:00`, while `TimeZoneDatabase::bundled()` (2026f) gives `-04:00`. Freshness depends on the source, so either keep the runtime image's `tzdata` package current or call `TimeZoneDatabase::bundled()` explicitly in the scheduler and refresh with `cargo update -p jiff-tzdb`. Whether `debian:bookworm-slim` and Lambda `provided.al2023` ship `/usr/share/zoneinfo` is **UNVERIFIED**; the bundled fallback covers both.
  - jiff is already in the tree when `rmcp` is added (`jiff ^1`, `std`), so it costs only `jiff-tzdb` on top of the MCP crate. Source: [rmcp 3.12.0 dependencies](https://crates.io/api/v1/crates/rmcp/3.12.0/dependencies).
- Fallback if we want a single date library: `croner` with its default `chrono` backend plus `chrono-tz = { version = "0.10.4", default-features = false }`, accepting tzdata 2025b until chrono-tz releases again.

**uuid.** 1.29.3, MSRV 1.89. Use `v7` for time-ordered primary keys (better B-tree locality than v4); Postgres 18's `uuidv7()` can also generate them as column defaults ([PostgreSQL docs](https://www.postgresql.org/docs/18/functions-uuid.html)). `Uuid::now_v7()` compiled (measured). Source: [crates.io uuid](https://crates.io/api/v1/crates/uuid).

### Secrets, Tokens, and Passwords

- **aes-gcm 0.11.1** (Apache-2.0 OR MIT, edition 2024). Defaults `aes`, `alloc`, `getrandom` are what we need. 0.11 moved to `aead` 0.6: `Nonce::generate()` and `Key::<Aes256Gcm>::generate()` replace the old `OsRng` calls; 96-bit nonces; the docs say the crate "has received one security audit by NCC Group, with no significant findings". Source: [aes-gcm docs](https://docs.rs/aes-gcm/0.11.1/aes_gcm/). Measured working pattern for `SECRET_KEY` (32 bytes, base64) and a stored `nonce || ciphertext`:

```rust
use aes_gcm::aead::{Aead, Generate, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};

let cipher = Aes256Gcm::new_from_slice(&key_bytes)?;     // 32 bytes from SECRET_KEY
let nonce = Nonce::generate();                           // unique per message
let mut sealed = nonce.to_vec();
sealed.extend(cipher.encrypt(&nonce, plaintext)?);
let (nonce_bytes, body) = sealed.split_at(12);
let opened = cipher.decrypt(&Nonce::try_from(nonce_bytes)?, body)?;
```

- **chacha20poly1305 0.11.0** (`XChaCha20Poly1305`, 192-bit nonce) is the alternative if random nonces under one key ever approach billions of messages. Copilot seals a few thousand provider keys and headers at most, so AES-256-GCM (as the design states) is fine. The 2^32 random-nonce guidance comes from NIST SP 800-38D and is cited from memory: **UNVERIFIED** in this session.
- **getrandom 0.4.3**: `getrandom::fill(&mut [u8; 32])` for session and API tokens (measured). `rand` is not needed directly (it is already in the tree via sqlx-postgres).
- **sha2 0.11.2** (published 2026-11-25): `Sha256::digest(token)` for stored token hashes and file content hashes. Already compiled for sqlx's `migrate` feature, so no extra crate.
- **base64 0.23.1**: `base64::engine::general_purpose::STANDARD` for `SECRET_KEY`, audio, and image parts. Default features are `std` and `simd-unsafe`; reqwest already enables them, so disabling them in our line would not change the build.
- **argon2 0.6.0** with `password-hash` 0.6.1:
  - `Params::DEFAULT_M_COST = 19 * 1024`, `DEFAULT_T_COST = 2`, `DEFAULT_P_COST = 1`, output 32 bytes, algorithm Argon2id v19. Source: [params.rs](https://docs.rs/crate/argon2/0.6.0/source/src/params.rs), [argon2 docs](https://docs.rs/argon2/0.6.0/argon2/).
  - OWASP lists "m=19456 (19 MiB), t=2, p=1" among its Argon2id minimum configurations (others: 46 MiB/t=1, 12 MiB/t=3, 9 MiB/t=4, 7 MiB/t=5). Source: [OWASP cheat sheet](https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html).
  - RFC 9106's recommended options are much heavier (t=1, p=4, 2 GiB; or t=3, p=4, 64 MiB). Source: [RFC 9106 section 4](https://www.rfc-editor.org/rfc/rfc9106.html#section-4). For a small self-hosted API, use `Argon2::default()` (OWASP 19 MiB); 64 MiB per concurrent login is an option for operators with memory to spare.
  - API in 0.6: `argon2.hash_password(password)?.to_string()` generates the salt itself (with `getrandom`) and returns a PHC string; verify with `verify_password(password, &PasswordHash::new(&stored)?)`, which uses the stored params, so params can be raised later with rehash-on-login (measured).
  - Run hashing in `tokio::task::spawn_blocking`; hash latency on our hardware is **UNVERIFIED** (not measured in release mode).

### axum Uploads, SSE, and Body Limits

- **Multipart:** axum 0.8.9 feature `multipart` (pulls `multer`). "For security reasons, by default, `Multipart` limits the request body size to 2MB." Raise it per route with `DefaultBodyLimit::max(..)`. Source: [axum multipart.rs](https://docs.rs/crate/axum/0.8.9/source/src/extract/multipart.rs), [DefaultBodyLimit](https://docs.rs/axum/0.8.9/axum/extract/struct.DefaultBodyLimit.html).
- **SSE:** `axum::response::sse::{Sse, Event, KeepAlive}` needs no extra feature. `Sse::keep_alive` needs the default `tokio` feature and `Event::json_data` the default `json` feature; the default keep-alive interval is 15 seconds. Source: [axum sse.rs](https://docs.rs/crate/axum/0.8.9/source/src/response/sse.rs).
- **Limits:** axum's docs: `DefaultBodyLimit` applies only to extractors that use it, while tower-http's `RequestBodyLimit` "is applied globally to all requests, regardless of which extractors are used or how the body is consumed". Source: [axum-core default_body_limit.rs](https://docs.rs/crate/axum-core/0.5.6/source/src/extract/default_body_limit.rs). tower-http 0.7.1 feature `limit` provides `RequestBodyLimitLayer`, which returns 413 early when `Content-Length` is too large. Source: [tower-http limit](https://docs.rs/tower-http/0.7.1/tower_http/limit/index.html). Plan: `DefaultBodyLimit::max` on the upload routes (for example 25 MiB) plus one global `RequestBodyLimitLayer` slightly above it as a backstop.
- **mime_guess 2.0.5:** maps file extensions only (no content sniffing). reqwest's `multipart` already depends on it with `default-features = false`; keep ours the same (the default `rev-mappings` feature is only needed for MIME-to-extension lookups). Use the client-sent content type first and `mime_guess::from_path(name).first_or_octet_stream()` as the fallback. Source: [crates.io mime_guess](https://crates.io/api/v1/crates/mime_guess), [reqwest Cargo.toml](https://github.com/seanmonstar/reqwest/blob/v0.13.5/Cargo.toml).

### wiremock (Dev)

- 0.6.5 (2025-08-24, MIT/Apache-2.0), depends on hyper 1, `tokio ^1.47.1`. Source: [wiremock 0.6.5 dependencies](https://crates.io/api/v1/crates/wiremock/0.6.5/dependencies).
- Measured: a `ResponseTemplate` with `content-type: text/event-stream` and a two-event body was parsed by `sse-stream` through a real reqwest client. wiremock sends the body as one response, so test chunk boundaries (an event split across reads) by feeding `SseStream::from_bytes_stream` a `futures_util::stream::iter` of byte chunks directly.

### Footprint (Measured)

`cargo tree --offline -e normal`, unique crates:

| Graph | Crates |
| --- | --- |
| Current workspace | 171 |
| Proposed set with reqwest `http2` on | 215 (+44) |
| Proposed set as in the lines above (no `http2`) | 213 (+42) |
| Plus `wiremock` (dev) | +14 |

Added crates, grouped: HTTP client (`reqwest`, `hyper-rustls`, `tokio-rustls`, `rustls-platform-verifier`, `rustls-native-certs`, `openssl-probe`, `ipnet`, `want`, `try-lock`, `tower-http` 0.6, `base64` 0.23, `tokio-util`), multipart (`multer`, `encoding_rs`, `simdutf8`, `spin`, `mime_guess`, `unicase`), crypto (`aes-gcm`, `aead`, `aes`, `cipher`, `ctr`, `ghash`, `polyval`, `universal-hash`, `inout`, `argon2`, `blake2`, `password-hash`, `phc`, `base64ct`, and small helpers), schedule (`croner`, `jiff`, `jiff-core`, `jiff-tzdb`), SSE (`sse-stream`). Release binary size and compile time were not measured.

## Web Packages

### Sizes (Measured)

esbuild 0.25.12, `--bundle --minify --format=esm`, React externalized, `NODE_ENV=production`, `gzip -9`. Rsbuild (Rspack) output will differ slightly. Weekly downloads from `api.npmjs.org` for 2026-11-19 to 2026-11-25.

| Package | Version | Minified | Gzip | Weekly downloads | Notes |
| --- | --- | --- | --- | --- | --- |
| Baseline: `react-dom/client` + `react-router-dom` 7 + `zod` 4 | | 774.6 KB | 193.8 KB | | for scale |
| `react-markdown` | 10.1.0 | 119.8 KB | 36.6 KB | 60.3 M | peer `react >=18` |
| `react-markdown` + `remark-gfm` | 10.1.0 + 4.0.1 | 166.4 KB | 50.4 KB | 75.5 M (gfm) | about 101 npm packages installed |
| + `rehype-sanitize` | 6.0.0 | 171.6 KB | 52.4 KB | | not needed (see below) |
| `marked` | 18.3.0 | 54.0 KB | 16.1 KB | 135.3 M | needs a sanitizer |
| `dompurify` | 3.5.0 | 32.2 KB | 12.5 KB | 104.1 M | |
| `streamdown` | 2.8.1 | 562.3 KB | 168.0 KB | 11.5 M | all chunks, many lazy |
| `highlight.js` core + 1 language | 11.12.0 | 29.1 KB | 11.3 KB | 57.6 M | BSD-3-Clause |
| `highlight.js` core + 10 languages | 11.12.0 | 69.4 KB | 22.1 KB | | |
| `lowlight` `common` | 3.3.0 | 172.9 KB | 52.4 KB | | |
| `rehype-highlight` | 7.0.2 | 180.7 KB | 55.2 KB | | uses lowlight `common` |
| `prism-react-renderer` | 2.4.1 | 86.2 KB | 26.4 KB | 7.2 M | |
| `sugar-high` `highlight` | 2.7.0 | 32.0 KB | 11.7 KB | 0.5 M | core only: 4.1 KB / 1.9 KB |
| `@speed-highlight/core` | 2.1.0 | 3.8 KB entry | 1.6 KB entry | 38.2 M | CC0-1.0; 70 lazy language chunks total 20.4 KB gzip |
| `shiki` core + JS regex engine + 1 theme + 1 language | 4.6.0 | 359.1 KB | 72.9 KB | 42.0 M | async init |
| `@tanstack/react-query` | 5.108.4 | 35.7 KB | 10.5 KB | 89.6 M | peer `react ^18 \|\| ^19` |
| `swr` (+ `swr/mutation`) | 2.5.1 | 15.5 KB | 7.0 KB | 27.0 M | |
| `eventsource-parser` (+ `/stream`) | 4.1.3 | 4.8 KB | 2.0 KB | 125.2 M | MIT, 1 package |
| `@microsoft/fetch-event-source` | 2.0.1 | 2.4 KB | 1.3 KB | 5.5 M | last publish 2021-04-25 |

Versions from `npm view <pkg> version` on 2026-11-26. A probe file using `react-markdown`, `remark-gfm`, `@tanstack/react-query`, `eventsource-parser/stream`, and `highlight.js/lib/core` type-checked with the repo's TypeScript 7.0.2 (measured).

### Markdown

- **Use `react-markdown` 10.1.0 + `remark-gfm` 4.0.1** (tables, task lists, strikethrough, autolinks). It renders to React elements and is "safe by default (no `dangerouslySetInnerHTML` or XSS attacks)", with `defaultUrlTransform` cleaning link URLs. Source: [react-markdown readme](https://github.com/remarkjs/react-markdown/blob/main/readme.md). Do not add `rehype-raw`, and then `rehype-sanitize` is unnecessary.
- `components` overrides let us render code blocks with a copy button and a language label without string HTML.
- Streaming cost: react-markdown re-parses the whole message on every update. Split the streaming message into blocks, memoize finished blocks, and re-render only the last block, throttled to animation frames. (Design guidance, not from a source.)
- Smaller but riskier: `marked` + `dompurify` is about 28.6 KB gzip, but needs `dangerouslySetInnerHTML` and loses per-block React reconciliation. `streamdown` handles unterminated Markdown during streaming but costs about 168 KB gzip across chunks.
- Route-level code splitting keeps the markdown chunk off the login and settings pages.

### Syntax Highlighting

- **v1: none.** Monospace block, language label, copy button. Zero bytes.
- **When added: `highlight.js` core with a curated language list loaded by dynamic `import()`**, run only after a code fence closes (not per token). 11 to 22 KB gzip measured, widely used, CSS themes, and `hljs.highlight(code, { language }).value` returns escaped HTML.
- Smallest option: `@speed-highlight/core` 2.1.0 (1.6 KB gzip entry, languages fetched on demand). It is DOM-oriented (`highlightElement`), so it needs a ref and effect in React.
- Avoid for this app: `shiki` (73 KB gzip minimum plus async engine setup), `lowlight`/`rehype-highlight` with `common` (52 to 55 KB), `prism-react-renderer` (26 KB, fixed language set).

### Server State: TanStack Query vs Plain Fetch

- The UI has many cached lists and mutations (chats with search, projects, files, connections, models, MCP servers, schedules) that need invalidation after create, rename, delete, and after a chat stream finishes (new title, updated time).
- **Recommendation: use `@tanstack/react-query` 5.108.4** (10.5 KB gzip) for every non-streaming call, wrapping the existing `apiFetch` from `apps/web/src/api/client.ts` as the `queryFn`. Hand-rolling caching, deduplication, invalidation, and retry would approach the same size with more bugs.
- `swr` 2.5.1 is smaller (7.0 KB) but its mutation and invalidation model is less explicit; the 3.5 KB difference is not worth it.
- Keep the chat stream outside react-query: a small custom hook that owns the fetch, the `AbortController`, and the in-progress message state, then invalidates the chat queries when the stream ends.

### SSE over POST

- `EventSource` cannot POST or set an `Authorization` header, so both `POST /chats/{id}/messages` and the reattach `GET /chats/{id}/stream` should use `fetch`.
- **Recommendation: `fetch` + `response.body.getReader()` + `TextDecoder` + `eventsource-parser`'s `createParser`** (2.0 KB gzip, 1 package, MIT), or a hand-written parser of about 40 lines. Either way, no retry or reconnect logic beyond what the app decides.
- Read with `reader.read()` in a loop, not `for await`: `ReadableStream` async iteration arrived in Chrome 124, Firefox 110, Safari 27. `pipeThrough` (Safari 10.1) and `TextDecoderStream` (Safari 14.1) are older. Source: [BCD ReadableStream](https://github.com/mdn/browser-compat-data/blob/main/api/ReadableStream.json), [BCD TextDecoderStream](https://github.com/mdn/browser-compat-data/blob/main/api/TextDecoderStream.json). Which Safari version a given WebKitGTK release matches is **UNVERIFIED**. Lead: the WebKit trunk change landed on 2025-12-05 as 303970@main ([WebKit bug 194379](https://bugs.webkit.org/show_bug.cgi?id=194379)), so WebKitGTK releases branched before then lack it; whether WebKitGTK 2.52 ships it enabled is **UNVERIFIED**.
- Do not use `@microsoft/fetch-event-source` 2.0.1: last published 2021-04-25 ([npm registry](https://registry.npmjs.org/@microsoft/fetch-event-source)), and its README says the connection "closes if the document is hidden ... and automatically retries with the last event ID when it becomes visible again" unless you opt out ([README](https://github.com/Azure/fetch-event-source/blob/main/README.md)). For a POST that creates a message, a silent retry duplicates the message.
- Stop: `AbortController.abort()` ends the local read; `POST /chats/{id}/stop` ends generation on the server.

### Push to Talk

- `MediaRecorder` costs 0 bytes (browser API). MDN marks it "Baseline: Widely available ... since April 2021". Source: [MDN MediaRecorder](https://developer.mozilla.org/en-US/docs/Web/API/MediaRecorder).
- Support: Chrome 47, Firefox 25, Safari 14.1, iOS Safari 14, Android WebView mirrors Chrome; `isTypeSupported()` from Safari 14.1. Source: [BCD MediaRecorder](https://github.com/mdn/browser-compat-data/blob/main/api/MediaRecorder.json). Pick the container with `MediaRecorder.isTypeSupported()` because Chromium and Safari record different formats.
- [voice.md](./voice.md) recommends encoding WAV (PCM16 mono) from an AudioWorklet instead, because OpenAI `input_audio` accepts only `wav` or `mp3` and Chromium's MediaRecorder produces WebM/Opus. That is a few dozen lines of TypeScript, no package. Platform notes (WebKitGTK permission handling, Android `RECORD_AUDIO`) are in voice.md and partly **UNVERIFIED** there.

### package.json Lines

For `apps/web/package.json` `dependencies`:

```json
"@tanstack/react-query": "^5.108.4",
"eventsource-parser": "^4.1.3",
"react-markdown": "^10.1.0",
"remark-gfm": "^4.0.1"
```

Only when syntax highlighting is added:

```json
"highlight.js": "^11.12.0"
```

## Verification Method

- Versions, release dates, MSRVs, licenses, feature maps, and dependency lists: crates.io API (`/api/v1/crates/<name>` and `/<version>/dependencies`) and `npm view`, on 2026-11-26.
- Rust: a throwaway crate in the session scratchpad (edition 2024, `rust-version = "1.94"`, rustc 1.97.0) with the exact lines above plus axum 0.8.9, tokio, sqlx 0.9.1 (same features as `copilot-database`), and tower-http 0.7.1, starting from the repo's `Cargo.lock`. `cargo check --all-targets` passed; `cargo test` passed a wiremock SSE test; small binaries exercised the ring provider install, AES-256-GCM seal and open, Argon2id hash and verify, `getrandom`, SHA-256, `Uuid::now_v7`, croner with jiff across a DST gap, and the bundled jiff tzdb. Crate counts from `cargo tree --offline -e normal`, duplicates from `cargo tree -d`.
- Web: packages installed in a scratch folder, bundled with esbuild 0.25.12 as described in [Sizes](#sizes-measured), and type-checked with the repo's TypeScript 7.0.2.
- No repository file other than this note was changed.

## Open Questions

- Lambda `provided.al2023`: CA bundle location for `rustls-platform-verifier` and presence of `/usr/share/zoneinfo` (**UNVERIFIED**).
- Whether `debian:bookworm-slim` includes `tzdata` by default (**UNVERIFIED**); decide between installing `tzdata` in `apps/api/Dockerfile` and forcing `TimeZoneDatabase::bundled()`.
- Argon2 hash latency and peak memory under concurrent logins on the target host (not measured).
- Release binary size and clean compile time of the proposed set (not measured).
- When `rmcp` moves to `sse-stream` 0.3, the duplicate 0.2.x copy disappears; re-check on rmcp bumps.

## Verification

Independent re-check on 2026-11-26 against crate tarballs from `static.crates.io`, the crates.io API, raw GitHub files, `npm view`, and small probe builds (rustc 1.97.0). All eight load-bearing claims held; only wording was tightened.

- reqwest 0.13.5: `Cargo.toml` has `rustls = ["__rustls-aws-lc-rs", ...]` and `rustls-no-provider` without it; `client.rs` uses `CryptoProvider::get_default()` and otherwise panics with the quoted message; CHANGELOG v0.13.0 says the provider "defaults to aws-lc instead of _ring_". sqlx-core 0.9.1 `tls_rustls.rs` uses `ring::default_provider()` under `_tls-rustls-ring-webpki`. Measured: `cargo tree -e normal` for reqwest (`rustls-no-provider`) + rustls (`ring`) + sqlx (repo features) contains `ring` 0.17.14 and no `aws-lc` crate. Confirmed.
- sqlx 0.9.1 (2026-11-18): features `chrono`, `json`, `uuid`, `time` exist; `rust_version` 1.94.0; `packages/database/Cargo.toml` enables `chrono`, `json`, `uuid`. Confirmed.
- Time zones: chrono-tz 0.10.4 (2025-07-11) is the latest and its `tz/NEWS` starts at 2025b; IANA NEWS tops at 2026f (2026-10-27) with Cuba staying on -04 after 2026-11-01; jiff-tzdb 1.0.0 `VERSION = "2026f"`; jiff 1.0.3 `from_env` tries zoneinfo (`TZDIR` or system), then concatenated, then the bundle. Measured: this container's tzdata is 2026e (`TZ=America/Havana date` gives -0500 for 2026-11-15) while `TimeZoneDatabase::bundled()` gives -04. Confirmed.
- croner 5.0.1 (2026-11-25): `Seconds` and `Year` default to `Optional`. Measured: the default parser accepts 6 and 7 fields; with both `Disallowed`, a 6-field pattern fails, and `L`, `15W`, `5#2`, `5#L`, `@daily`, `@hourly` parse; `30 2 * * *` in `America/New_York` fires 2026-03-08T03:00-04:00, then 02:30. cron 0.17.0 rejected every 5-field pattern tried (`30 9 * * *`, `*/5 * * * *`, `30 9 * * MON-FRI`) and `L` in day-of-month, and accepted a 6-field pattern. Confirmed.
- argon2 0.6.0: `DEFAULT_M_COST = 19 * 1024`, `T = 2`, `P = 1`, output 32; `Argon2::default()` is Argon2id V0x13 (measured). The OWASP cheat sheet lists "m=19456 (19 MiB), t=2, p=1". Confirmed; wording changed because `Params` does not carry the algorithm.
- sse-stream 0.3.0 CHANGELOG matches (unknown fields ignored, error once then end, 64 KiB retained scratch does not bound an in-progress event). The CRLF item in the changelog is about encoding; the decoder's own tests cover CRLF and CR input. rmcp 3.12.0 (released 2026-11-26) declares `sse-stream` 0.2.4, optional, via `client-side-sse` and `server-side-http`. Confirmed; wording clarified, plus a new note on rmcp features that would pull aws-lc-rs.
- `@microsoft/fetch-event-source`: npm `time` shows 2.0.1 on 2021-04-25 as the last publish; `src/fetch.ts` aborts on every `visibilitychange` and calls `create()` again with the same init (method and body) unless `openWhenHidden`. Confirmed.
- BCD `api/ReadableStream.json` `@@asyncIterator` and `values`: Chrome 124, Firefox 110, Safari 27 (Safari 27 released 2026-09-14 per BCD `browsers/safari.json`). Confirmed; WebKitGTK mapping remains **UNVERIFIED**.
- Not re-checked here: the measured crate counts, web bundle sizes, and download numbers.

## P6.1 Trim

The final review removed what no crate used: the workspace entries `futures-core`, `mime_guess`, and `tokio-stream`, `tokio-util` in `copilot-providers`, and reqwest's `multipart` feature (only the API reads multipart, through axum). `idna_adapter` is pinned to 1.1 in `copilot-core`, which keeps `url` on the unicode-rs IDNA back end and drops the ICU4X crates (twenty fewer packages in `Cargo.lock`). `sse-stream` stays at two versions (0.3 in `copilot-providers`, 0.2.6 through rmcp): moving the providers to 0.2 is not worth the risk before the first release.
