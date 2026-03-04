use async_trait::async_trait;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::context::Context;
use crate::error::{Error, Result};
use crate::step::{ErrorStrategy, Step};

// ── Guard ──────────────────────────────────────────────────────────

type GuardFn = Arc<dyn Fn(&Context) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> + Send + Sync>;

/// Runs a step only when a condition is true, optionally with a fallback.
///
/// ```ignore
/// Guard::new("check_part_present", inner_step, |ctx| Box::pin(async move {
///     ctx.get_bool("part_detected").await
/// }))
/// ```
pub struct Guard {
    name: String,
    inner: Box<dyn Step>,
    condition: GuardFn,
    fallback: Option<Box<dyn Step>>,
}

impl Guard {
    pub fn new<F>(name: impl Into<String>, step: impl Step + 'static, condition: F) -> Self
    where
        F: Fn(&Context) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> + Send + Sync + 'static,
    {
        Self {
            name: name.into(),
            inner: Box::new(step),
            condition: Arc::new(condition),
            fallback: None,
        }
    }

    pub fn with_fallback(mut self, step: impl Step + 'static) -> Self {
        self.fallback = Some(Box::new(step));
        self
    }
}

#[async_trait]
impl Step for Guard {
    fn name(&self) -> &str {
        &self.name
    }

    async fn run(&self, ctx: &Context) -> Result<()> {
        if (self.condition)(ctx).await {
            self.inner.run(ctx).await
        } else if let Some(fb) = &self.fallback {
            fb.run(ctx).await
        } else {
            Err(Error::GuardFailed {
                step: self.name.clone(),
            })
        }
    }

    async fn compensate(&self, ctx: &Context) -> Result<()> {
        // Always compensate the inner step — it ran if we got here.
        // Do NOT re-evaluate the condition.
        self.inner.compensate(ctx).await
    }
}

// ── Branch ─────────────────────────────────────────────────────────

type SelectorFn = Arc<
    dyn Fn(&Context) -> Pin<Box<dyn Future<Output = usize> + Send + '_>> + Send + Sync,
>;

/// Select one of several steps to run based on a selector function.
///
/// The selector returns an index into the branches vector.
/// Only the taken branch is compensated.
pub struct Branch {
    name: String,
    branches: Vec<Box<dyn Step>>,
    selector: SelectorFn,
    /// Tracks which branch was taken (set during run, read during compensate).
    taken: Mutex<Option<usize>>,
}

impl Branch {
    pub fn new<F>(name: impl Into<String>, selector: F) -> Self
    where
        F: Fn(&Context) -> Pin<Box<dyn Future<Output = usize> + Send + '_>> + Send + Sync + 'static,
    {
        Self {
            name: name.into(),
            branches: Vec::new(),
            selector: Arc::new(selector),
            taken: Mutex::new(None),
        }
    }

    pub fn branch(mut self, step: impl Step + 'static) -> Self {
        self.branches.push(Box::new(step));
        self
    }
}

#[async_trait]
impl Step for Branch {
    fn name(&self) -> &str {
        &self.name
    }

    async fn run(&self, ctx: &Context) -> Result<()> {
        let idx = (self.selector)(ctx).await;
        if idx < self.branches.len() {
            *self.taken.lock().await = Some(idx);
            self.branches[idx].run(ctx).await
        } else {
            Err(Error::TaskFailed {
                task: self.name.clone(),
                message: format!("branch index {idx} out of range (have {})", self.branches.len()),
            })
        }
    }

    async fn compensate(&self, ctx: &Context) -> Result<()> {
        // Only compensate the branch that was actually taken
        if let Some(idx) = *self.taken.lock().await {
            if idx < self.branches.len() {
                self.branches[idx].compensate(ctx).await?;
            }
        }
        Ok(())
    }
}

// ── Loop ───────────────────────────────────────────────────────────

type CondFn = Arc<
    dyn Fn(&Context) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> + Send + Sync,
>;

/// Repeat a step while a condition holds.
///
/// Supports an optional max iteration count for safety.
/// On compensation, the body is compensated once for each completed
/// iteration (in reverse order).
///
/// ```ignore
/// Loop::new("pick_all_parts", pick_step, |ctx| Box::pin(async move {
///     ctx.get_bool("parts_remaining").await
/// }))
/// .max_iterations(50)
/// ```
pub struct Loop {
    name: String,
    body: Box<dyn Step>,
    condition: CondFn,
    max_iterations: Option<u32>,
    /// Number of completed iterations (set during run, read during compensate).
    completed_iterations: AtomicU32,
}

impl Loop {
    pub fn new<F>(name: impl Into<String>, body: impl Step + 'static, condition: F) -> Self
    where
        F: Fn(&Context) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> + Send + Sync + 'static,
    {
        Self {
            name: name.into(),
            body: Box::new(body),
            condition: Arc::new(condition),
            max_iterations: None,
            completed_iterations: AtomicU32::new(0),
        }
    }

    pub fn max_iterations(mut self, n: u32) -> Self {
        self.max_iterations = Some(n);
        self
    }
}

#[async_trait]
impl Step for Loop {
    fn name(&self) -> &str {
        &self.name
    }

    async fn run(&self, ctx: &Context) -> Result<()> {
        self.completed_iterations.store(0, Ordering::SeqCst);
        let mut count = 0u32;

        while (self.condition)(ctx).await {
            ctx.check_cancelled()?;

            match self.body.run(ctx).await {
                Ok(()) => {
                    count += 1;
                    self.completed_iterations.store(count, Ordering::SeqCst);
                }
                Err(e) => {
                    // Body failed — compensate all completed iterations
                    // before propagating the error (saga pattern).
                    ctx.enter_compensation();
                    let mut comp_errs = Vec::new();
                    for _ in (0..count).rev() {
                        if let Err(ce) = self.body.compensate(ctx).await {
                            comp_errs.push(ce);
                        }
                    }
                    ctx.exit_compensation();
                    // Reset counter since we've already compensated
                    self.completed_iterations.store(0, Ordering::SeqCst);
                    return Err(e);
                }
            }

            if let Some(max) = self.max_iterations {
                if count >= max {
                    break;
                }
            }
        }

        Ok(())
    }

    async fn compensate(&self, ctx: &Context) -> Result<()> {
        // Compensate once for each completed iteration, in reverse.
        let n = self.completed_iterations.load(Ordering::SeqCst);
        for _ in (0..n).rev() {
            self.body.compensate(ctx).await?;
        }
        Ok(())
    }

    fn error_strategy(&self, error: &Error) -> ErrorStrategy {
        self.body.error_strategy(error)
    }
}
