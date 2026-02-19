use async_trait::async_trait;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

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
pub struct Branch {
    name: String,
    branches: Vec<Box<dyn Step>>,
    selector: SelectorFn,
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
            self.branches[idx].run(ctx).await
        } else {
            Err(Error::TaskFailed {
                task: self.name.clone(),
                message: format!("branch index {idx} out of range (have {})", self.branches.len()),
            })
        }
    }

    async fn compensate(&self, ctx: &Context) -> Result<()> {
        // Compensate all branches (only the one that ran has state, others are no-ops)
        for step in self.branches.iter().rev() {
            step.compensate(ctx).await?;
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
        let mut count = 0u32;

        while (self.condition)(ctx).await {
            ctx.check_cancelled()?;

            self.body.run(ctx).await?;

            count += 1;
            if let Some(max) = self.max_iterations {
                if count >= max {
                    break;
                }
            }
        }

        Ok(())
    }

    async fn compensate(&self, ctx: &Context) -> Result<()> {
        self.body.compensate(ctx).await
    }

    fn error_strategy(&self, error: &Error) -> ErrorStrategy {
        self.body.error_strategy(error)
    }
}
