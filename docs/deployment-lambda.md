# AWS Lambda Deployment (Optional)

This template ships an optional Lambda entrypoint for serverless API deployment. The default workflow uses the long-running `server` binary (see [deployment-container.md](./deployment-container.md)).

## When to Use Lambda

| Approach | Use when |
| --- | --- |
| **`server`** (default) | Local dev, containers, VMs, always-on hosting |
| **`lambda`** | AWS Lambda + API Gateway, pay-per-request, no server management |

If you will never deploy to Lambda, delete `apps/api/src/bin/lambda.rs`, the `lambda` feature, the `[[bin]] lambda` entry, and the `lambda_http` dependency in `apps/api/Cargo.toml`, and remove this document.

## Handler

[`apps/api/src/bin/lambda.rs`](../apps/api/src/bin/lambda.rs) builds the same app as the server through `ApiContext::create()` once per cold start; warm invocations reuse the app and the connection pool.

Build it with [cargo-lambda](https://www.cargo-lambda.info/):

```bash
cargo lambda build -p copilot-api --bin lambda --features lambda --release --arm64
```

The artifact lands in `target/lambda/lambda/bootstrap`. Use the `provided.al2023` runtime.

## Environment Variables

Use the same variables as the local API. See [env.md](./env.md).

| Variable | Lambda note |
| --- | --- |
| `ENVIRONMENT=production` | Turns database SSL on and requires `AUTH_DISABLED=false` |
| `DB_*` | Point at your RDS / Aurora / external Postgres; set `DB_SSL_ROOT_CERT` to the provider's CA bundle when it is not in the Mozilla roots |
| `LOG_LEVEL` | Use `info` or `warn` in production; logs are JSON |

Inject `DB_PASSWORD` from Secrets Manager or SSM through your deploy tooling; the template does not read AWS secret stores itself.

## Connection Pooling

A warm Lambda container reuses the pool created in `ApiContext::create()` (`DB_POOL_MAX` is 4).

- Use **RDS Proxy** between Lambda and Postgres for connection multiplexing.
- Keep the pool small when connecting directly without a proxy.

## Migrations

Run the `migrate` binary as a separate step before deploying new code (a CI job with network access to the database, a one-off container task, or a small Lambda that wraps `copilot_database::operations::migrate::run`). See [ADR-002](./adr/002-database-migrations-and-seed.md).

## API Gateway

Expose the function through an HTTP API or REST API:

| Route | Purpose |
| --- | --- |
| `GET /health` | Liveness probe |
| `GET /health/ready` | Readiness probe (checks Postgres) |

Map stage paths to the integration. No path prefix is required unless API Gateway adds one.
