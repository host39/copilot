//! Port for CPU-bound work that must not hold up the tasks serving requests.

use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;

/// One piece of CPU-bound work.
pub type Job = Box<dyn FnOnce() + Send + 'static>;

/// Runs CPU-bound work (hashing an upload, counting PDF pages, measuring audio) where it cannot
/// stall the executor: the API runs jobs on Tokio's blocking pool. A job that panics resumes
/// the panic in the caller.
#[async_trait]
pub trait Worker: Send + Sync {
    async fn run(&self, job: Job);
}

/// Runs each job in place, on the caller's task. For tests and tools without a runtime.
#[derive(Clone, Copy, Debug, Default)]
pub struct InlineWorker;

#[async_trait]
impl Worker for InlineWorker {
    async fn run(&self, job: Job) {
        job();
    }
}

/// Runs `job` on `worker` and returns its result.
pub async fn offload<T, F>(worker: &dyn Worker, job: F) -> T
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let slot = Arc::new(Mutex::new(None));
    let filled = Arc::clone(&slot);
    worker
        .run(Box::new(move || {
            let value = job();
            *filled.lock().unwrap_or_else(PoisonError::into_inner) = Some(value);
        }))
        .await;
    slot.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
        .expect("a worker runs every job to completion or resumes its panic")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn returns_the_value_of_the_job() {
        let doubled = offload(&InlineWorker, || 21 * 2).await;
        assert_eq!(doubled, 42);
    }
}
