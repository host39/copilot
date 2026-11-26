# Rust Imports

The Rust counterpart of `import-paths`: one absolute way to name every item.

- **Within a crate**: absolute `crate::` paths in `use` and `pub use` (`use crate::config::AppConfig;`).
- **Never `super::`** or bare relative module paths. The one exception is `use super::*;` as the first line of a colocated `#[cfg(test)] mod tests`. `scripts/lint-rust-imports.sh` (part of `make lint`) enforces the `super::` part.
- **Cross-crate**: the crate name (`use copilot_core::DomainError;`), declared in the crate's `Cargo.toml` as `copilot-core = { workspace = true }`.
- **Barrels**: re-export each crate's public API from `lib.rs`, and each module's from its `mod.rs` (the Rust equivalent of `index.ts`). Import from the crate root when a re-export exists.
- **No glob imports** (`use some::module::*;`) outside test modules.
- **Grouping**: `std`, then external and workspace crates, then `crate::`, with one blank line between groups. rustfmt sorts within each group.

```rust
// BAD
use super::config::AppConfig;
use self::routes::health;
use copilot_core::shared::errors::DomainError; // a deep path when the root re-exports it

// GOOD
use crate::config::AppConfig;
use crate::routes::health;
use copilot_core::DomainError;
```

New crates: add the path to `[workspace.dependencies]` in the root `Cargo.toml`, depend on it with `{ workspace = true }`, and add its rules to `boundaries.config.mjs`.
