//! `sqlx` implementations of the repository traits (ports) defined in `copilot-core`.
//!
//! Add one module per aggregate, for example `widgets/sqlx_widget_repository.rs` exposing
//! `SqlxWidgetRepository`, and re-export it here. Map driver errors with
//! `DomainError::internal("database.<area>.<operation>.failed", error)`.
