// Dependency boundaries between workspace crates, checked by `make boundaries`
// (scripts/boundaries.mjs). Cargo already limits a crate to what its Cargo.toml lists; these
// rules stop the wrong entries from being added there.
//
// `forbidden`: `from` is a crate name; `to` lists crates it must not depend on directly (normal or
// build dependencies; dev-dependencies are only compiled for tests and are not checked). A `to`
// entry may name a crate that is not a dependency of anything yet.
//
// `limited`: `from` may depend on `to` only with default features off and no features beyond
// `features`.
//
// Identity comes from lib-rustauth, through its `rustauth` facade only (the inner crates may
// change in any release). Only `apps/api` and `copilot-database` may depend on it: `apps/api`
// with the `axum` and `postgres` features (and only in `helpers/auth.rs`, which a test checks),
// `copilot-database` only with the `migrate` feature, for the `auth` migrations.

const RUSTAUTH = [
  "rustauth",
  "rustauth-axum",
  "rustauth-contracts",
  "rustauth-core",
  "rustauth-crypto",
  "rustauth-postgres",
];

const ADAPTERS = ["copilot-mcp", "copilot-providers", "copilot-security"];

/** Every crate that is part of the product (the image builds from these). */
const PRODUCT = [
  "copilot-api",
  "copilot-contracts",
  "copilot-core",
  "copilot-database",
  "copilot-logger",
  "copilot-middleware",
  ...ADAPTERS,
];

/** Dev-only binaries under `tools/` (never in the image). */
const TOOLS = ["fake-llm"];

/** Outbound adapters implement core ports only: no HTTP server, database, or identity crates. */
function adapterRule(crate) {
  return {
    name: `${crate.replace("copilot-", "")}-adapter-only`,
    severity: "error",
    comment: `${crate} implements core ports only: no apps, database, contracts, middleware, other adapters, rustauth, axum, sqlx, or utoipa`,
    from: crate,
    to: [
      "copilot-api",
      "copilot-contracts",
      "copilot-database",
      "copilot-middleware",
      ...ADAPTERS.filter((adapter) => adapter !== crate),
      ...RUSTAUTH,
      "axum",
      "sqlx",
      "utoipa",
    ],
  };
}

/**
 * @typedef {{ name: string, severity: "error" | "warn", comment: string, from: string, to: string[] }} Forbidden
 * @typedef {{ name: string, severity: "error" | "warn", comment: string, from: string, to: string, features: string[] }} Limited
 * @type {{ forbidden: Forbidden[], limited: Limited[] }}
 */
export default {
  forbidden: [
    {
      name: "core-no-infrastructure",
      severity: "error",
      comment:
        "packages/core must not depend on adapters, apps, contracts, identity, or HTTP/database/crypto/logging implementations",
      from: "copilot-core",
      to: [
        "copilot-api",
        "copilot-contracts",
        "copilot-database",
        "copilot-logger",
        "copilot-middleware",
        ...ADAPTERS,
        ...RUSTAUTH,
        "aes-gcm",
        "argon2",
        "axum",
        "hyper",
        "reqwest",
        "rmcp",
        "sqlx",
        "sse-stream",
        "tokio",
        "tower",
        "tower-http",
        "tracing-subscriber",
        "utoipa",
      ],
    },
    ...ADAPTERS.map(adapterRule),
    {
      name: "database-no-apps",
      severity: "error",
      comment:
        "packages/database must not depend on apps, adapters, HTTP crates, or rustauth's inner crates (the rustauth facade runs the auth migrations)",
      from: "copilot-database",
      to: [
        "copilot-api",
        "copilot-middleware",
        ...ADAPTERS,
        ...RUSTAUTH.filter((crate) => crate !== "rustauth"),
        "axum",
        "tower-http",
        "utoipa",
      ],
    },
    {
      name: "middleware-no-database",
      severity: "error",
      comment: "packages/middleware must not depend on packages/database, adapters, or rustauth",
      from: "copilot-middleware",
      to: ["copilot-database", ...ADAPTERS, ...RUSTAUTH, "sqlx"],
    },
    {
      name: "logger-no-database",
      severity: "error",
      comment: "packages/logger must not depend on packages/database",
      from: "copilot-logger",
      to: ["copilot-database", "sqlx"],
    },
    {
      name: "api-rustauth-facade-only",
      severity: "error",
      comment:
        "apps/api uses lib-rustauth through its rustauth facade only, never the inner crates",
      from: "copilot-api",
      to: RUSTAUTH.filter((crate) => crate !== "rustauth"),
    },
    {
      name: "contracts-no-infrastructure",
      severity: "error",
      comment:
        "packages/contracts must stay transport-agnostic (no core, database, adapters, apps, rustauth, axum, or sqlx)",
      from: "copilot-contracts",
      to: [
        "copilot-api",
        "copilot-core",
        "copilot-database",
        ...ADAPTERS,
        ...RUSTAUTH,
        "axum",
        "sqlx",
      ],
    },
    {
      name: "fake-llm-standalone",
      severity: "error",
      comment:
        "tools/fake-llm is a dev-only fake of the providers and an MCP server: axum, tokio, serde, rmcp and the like only, never a Copilot crate, rustauth, or sqlx",
      from: "fake-llm",
      to: [...PRODUCT, ...RUSTAUTH, "sqlx"],
    },
    ...PRODUCT.map((crate) => ({
      name: `${crate.replace("copilot-", "")}-no-dev-tools`,
      severity: "error",
      comment: `${crate} must not depend on the dev-only tools/ crates, so they stay out of the release build and the image`,
      from: crate,
      to: TOOLS,
    })),
  ],
  limited: [
    {
      name: "api-rustauth-axum-postgres",
      severity: "error",
      comment:
        'apps/api uses the rustauth facade for its router, guard, and Postgres storage: default-features = false, features = ["axum", "postgres"] (dev-dependencies may add "testing")',
      from: "copilot-api",
      to: "rustauth",
      features: ["axum", "postgres"],
    },
    {
      name: "database-rustauth-migrate-only",
      severity: "error",
      comment:
        'packages/database uses the rustauth facade for the auth migrations only: default-features = false, features = ["migrate"]',
      from: "copilot-database",
      to: "rustauth",
      features: ["migrate"],
    },
  ],
};
