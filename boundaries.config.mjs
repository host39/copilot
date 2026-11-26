// Dependency boundaries between workspace crates, checked by `make boundaries`
// (scripts/boundaries.mjs). Cargo already limits a crate to what its Cargo.toml lists; these
// rules stop the wrong entries from being added there.
//
// `from` is a crate name; `to` lists crates it must not depend on directly (normal or build
// dependencies; dev-dependencies are only compiled for tests and are not checked).

/** @type {{ forbidden: { name: string, severity: "error" | "warn", comment: string, from: string, to: string[] }[] }} */
export default {
  forbidden: [
    {
      name: "core-no-infrastructure",
      severity: "error",
      comment:
        "packages/core must not depend on database, apps, contracts, or HTTP/database/logging implementations",
      from: "copilot-core",
      to: [
        "copilot-api",
        "copilot-contracts",
        "copilot-database",
        "copilot-logger",
        "copilot-middleware",
        "axum",
        "hyper",
        "lambda_http",
        "sqlx",
        "tower",
        "tower-http",
        "tracing-subscriber",
        "utoipa",
      ],
    },
    {
      name: "database-no-apps",
      severity: "error",
      comment: "packages/database must not depend on apps or HTTP crates",
      from: "copilot-database",
      to: ["copilot-api", "copilot-middleware", "axum", "tower-http", "utoipa"],
    },
    {
      name: "middleware-no-database",
      severity: "error",
      comment: "packages/middleware must not depend on packages/database",
      from: "copilot-middleware",
      to: ["copilot-database", "sqlx"],
    },
    {
      name: "logger-no-database",
      severity: "error",
      comment: "packages/logger must not depend on packages/database",
      from: "copilot-logger",
      to: ["copilot-database", "sqlx"],
    },
    {
      name: "contracts-no-infrastructure",
      severity: "error",
      comment: "packages/contracts must stay transport-agnostic (no database, apps, axum, or sqlx)",
      from: "copilot-contracts",
      to: ["copilot-api", "copilot-database", "axum", "sqlx"],
    },
  ],
};
