//! AWS Lambda entrypoint (optional). Build with `cargo lambda build -p copilot-api --bin lambda
//! --features lambda --release`, or delete this file and the `lambda` feature for a server-only
//! deployment. See docs/deployment-lambda.md.
//!
//! The context is created once per cold start; warm invocations reuse the app and pool.

use copilot_api::helpers::bootstrap::ApiContext;

#[tokio::main]
async fn main() -> Result<(), lambda_http::Error> {
    let context = ApiContext::create()?;
    lambda_http::run(context.app).await
}
