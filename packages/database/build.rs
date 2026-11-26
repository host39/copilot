//! Rebuilds the crate when a migration is added or removed. `sqlx::migrate!` embeds each file
//! with `include_str!`, so edits are picked up, but on stable Rust it cannot watch the
//! directory for new files.

// Cargo reads build script instructions from standard output.
#[allow(clippy::print_stdout)]
fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
