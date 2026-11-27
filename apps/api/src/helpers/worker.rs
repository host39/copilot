//! The [`Worker`] adapter: CPU-bound work on Tokio's blocking pool.

use async_trait::async_trait;
use copilot_core::{Job, Worker};

/// Runs jobs with `spawn_blocking`, so hashing or measuring a large upload never stops the
/// runtime from serving other requests. A panicking job resumes its panic in the caller.
#[derive(Clone, Copy, Debug, Default)]
pub struct BlockingPool;

#[async_trait]
impl Worker for BlockingPool {
    async fn run(&self, job: Job) {
        if let Err(error) = tokio::task::spawn_blocking(job).await
            && error.is_panic()
        {
            std::panic::resume_unwind(error.into_panic());
        }
    }
}

#[cfg(test)]
mod tests {
    use copilot_core::offload;

    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn runs_jobs_off_the_runtime_thread() {
        let caller = std::thread::current().id();
        let worker = offload(&BlockingPool, || std::thread::current().id()).await;
        assert_ne!(caller, worker);
    }
}
