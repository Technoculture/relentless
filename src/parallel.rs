use async_trait::async_trait;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};

use crate::context::Context;
use crate::error::{Error, Result};
use crate::step::{ErrorStrategy, Step};

/// Execute steps concurrently; if any fails, signal cancellation to
/// siblings and compensate all that succeeded.
///
/// All steps share the same `Context`, which is safe because it uses
/// interior mutability (`Arc<RwLock<…>>`).
pub struct Parallel {
    name: String,
    steps: Vec<Box<dyn Step>>,
}

impl Parallel {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            steps: Vec::new(),
        }
    }

    pub fn step(mut self, s: impl Step + 'static) -> Self {
        self.steps.push(Box::new(s));
        self
    }
}

/// Poll all futures concurrently. When the first error is seen,
/// set a flag so the wrapping futures can bail out quickly.
struct JoinAll<'a> {
    futs: Vec<Option<Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>>>,
    results: Vec<Option<Result<()>>>,
    failed: Arc<AtomicBool>,
}

impl<'a> Future for JoinAll<'a> {
    type Output = Vec<Result<()>>;

    fn poll(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Self::Output> {
        // SAFETY: we never move the inner futures, only poll them in place.
        let this = unsafe { self.get_unchecked_mut() };
        let mut all_done = true;

        for i in 0..this.futs.len() {
            if let Some(fut) = &mut this.futs[i] {
                match fut.as_mut().poll(cx) {
                    Poll::Ready(result) => {
                        if result.is_err() {
                            this.failed.store(true, Ordering::SeqCst);
                        }
                        this.results[i] = Some(result);
                        this.futs[i] = None;
                    }
                    Poll::Pending => {
                        all_done = false;
                    }
                }
            }
        }

        if all_done {
            let results = this.results.iter_mut().map(|r| r.take().unwrap()).collect();
            Poll::Ready(results)
        } else {
            Poll::Pending
        }
    }
}

#[async_trait]
impl Step for Parallel {
    fn name(&self) -> &str {
        &self.name
    }

    async fn run(&self, ctx: &Context) -> Result<()> {
        if self.steps.is_empty() {
            return Ok(());
        }

        // Shared flag: set to true when any step fails, so siblings can bail.
        let failed = Arc::new(AtomicBool::new(false));

        let futs: Vec<_> = self
            .steps
            .iter()
            .map(|s| {
                let f = failed.clone();
                Some(Box::pin(async move {
                    // Check if a sibling already failed before we start heavy work
                    if f.load(Ordering::SeqCst) {
                        return Err(Error::Cancelled);
                    }
                    let result = s.run(ctx).await;
                    if result.is_err() {
                        f.store(true, Ordering::SeqCst);
                    }
                    result
                }) as Pin<Box<dyn Future<Output = Result<()>> + Send + '_>>)
            })
            .collect();

        let results: Vec<Option<Result<()>>> = (0..futs.len()).map(|_| None).collect();
        let all_results = JoinAll {
            results,
            futs,
            failed,
        }
        .await;

        // Partition into succeeded / failed
        let mut succeeded = Vec::new();
        let mut first_error: Option<(String, Error)> = None;

        for (i, result) in all_results.into_iter().enumerate() {
            match result {
                Ok(()) => succeeded.push(i),
                Err(e) => {
                    // Skip cancellation errors from siblings — find the real error
                    if first_error.is_none() && !e.is_cancelled() {
                        first_error = Some((self.steps[i].name().to_string(), e));
                    } else if first_error.is_none() {
                        // If all errors are cancellations, use the first one
                        first_error = Some((self.steps[i].name().to_string(), e));
                    }
                }
            }
        }

        if let Some((failed_step, err)) = first_error {
            let mut comp_errors = Vec::new();
            for &i in succeeded.iter().rev() {
                if let Err(ce) = self.steps[i].compensate(ctx).await {
                    comp_errors.push(ce);
                }
            }
            Err(Error::ParallelFailed {
                failed_step,
                source: Box::new(err),
                compensation_errors: comp_errors,
            })
        } else {
            Ok(())
        }
    }

    async fn compensate(&self, ctx: &Context) -> Result<()> {
        let mut errors = Vec::new();
        for step in self.steps.iter().rev() {
            if let Err(e) = step.compensate(ctx).await {
                errors.push(e);
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(Error::ParallelFailed {
                failed_step: self.name.clone(),
                source: Box::new(Error::TaskFailed {
                    task: self.name.clone(),
                    message: "compensation requested".into(),
                }),
                compensation_errors: errors,
            })
        }
    }

    fn error_strategy(&self, _error: &Error) -> ErrorStrategy {
        ErrorStrategy::Compensate
    }
}
