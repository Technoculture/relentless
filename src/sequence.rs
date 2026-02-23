use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

use crate::context::Context;
use crate::error::{Error, Result};
use crate::retry::RetryPolicy;
use crate::step::{ErrorStrategy, Step};

/// Default timeout applied to each individual compensation step.
const COMPENSATION_TIMEOUT: Duration = Duration::from_secs(30);

/// Linear execution with automatic reverse-order compensation on failure.
///
/// Supports: retry, timeout, cancellation, hooks, journaling,
/// and per-step error discrimination (Compensate / Skip / Escalate).
pub struct Sequence {
    name: String,
    steps: Vec<Box<dyn Step>>,
    retry: RetryPolicy,
    timeout: Option<Duration>,
    compensation_timeout: Duration,
}

impl Sequence {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            steps: Vec::new(),
            retry: RetryPolicy::none(),
            timeout: None,
            compensation_timeout: COMPENSATION_TIMEOUT,
        }
    }

    pub fn step(mut self, s: impl Step + 'static) -> Self {
        self.steps.push(Box::new(s));
        self
    }

    pub fn retry(mut self, policy: RetryPolicy) -> Self {
        self.retry = policy;
        self
    }

    pub fn timeout(mut self, dur: Duration) -> Self {
        self.timeout = Some(dur);
        self
    }

    pub fn compensation_timeout(mut self, dur: Duration) -> Self {
        self.compensation_timeout = dur;
        self
    }

    /// Execute the sequence, tracking completed steps in shared state
    /// so that timeout/cancellation can compensate them.
    async fn execute(&self, ctx: &Context, completed: Arc<Mutex<Vec<usize>>>) -> Result<()> {
        for (i, step) in self.steps.iter().enumerate() {
            // Check cancellation between steps; compensate if cancelled
            if ctx.cancel.is_cancelled() {
                let comp = completed.lock().await;
                let comp_errs = self.compensate_completed(ctx, &comp).await;
                return Err(Error::SequenceFailed {
                    failed_step: self.name.clone(),
                    source: Box::new(Error::Cancelled),
                    compensation_errors: comp_errs,
                });
            }

            let step_name = step.name().to_string();
            ctx.hooks.fire_start(&step_name, ctx);

            // Journal: record started
            if let Some(j) = &ctx.journal {
                let _ = j.record_started(&ctx.workflow_id, &step_name).await;
            }

            // Skip already-completed steps (crash recovery).
            // Use count-based matching to handle duplicate step names:
            // if journal shows 2 "place_resistor" completed and we're at
            // the 3rd "place_resistor", we know it hasn't run yet.
            if let Some(j) = &ctx.journal {
                let done = j.completed_steps(&ctx.workflow_id).await.unwrap_or_default();
                let done_count = done.iter().filter(|s| s.as_str() == step_name).count();
                let prior_same = self.steps[..i].iter().filter(|s| s.name() == step_name).count();
                if done_count > prior_same {
                    completed.lock().await.push(i);
                    continue;
                }
            }

            // Run with retry, but consult error_strategy on each failure
            let mut last_err = None;
            for attempt in 1..=self.retry.max_attempts {
                let delay = self.retry.delay_for_attempt(attempt);
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }

                match step.run(ctx).await {
                    Ok(()) => {
                        last_err = None;
                        break;
                    }
                    Err(e) => {
                        // Check error strategy BEFORE retrying.
                        // Escalate and Skip should not be retried.
                        let strategy = step.error_strategy(&e);
                        if strategy != ErrorStrategy::Compensate {
                            last_err = Some((e, strategy));
                            break;
                        }
                        last_err = Some((e, strategy));
                    }
                }
            }

            if let Some((err, strategy)) = last_err {
                ctx.hooks.fire_error(&step_name, ctx, &err);

                if let Some(j) = &ctx.journal {
                    let _ = j
                        .record_failed(&ctx.workflow_id, &step_name, &err.to_string())
                        .await;
                }

                match strategy {
                    ErrorStrategy::Compensate => {
                        let comp = completed.lock().await;
                        let comp_errs = self.compensate_completed(ctx, &comp).await;
                        return Err(Error::SequenceFailed {
                            failed_step: step_name,
                            source: Box::new(err),
                            compensation_errors: comp_errs,
                        });
                    }
                    ErrorStrategy::Skip => {
                        // Skip this step, keep going
                        continue;
                    }
                    ErrorStrategy::Escalate => {
                        // Stop immediately, no compensation
                        return Err(Error::SequenceFailed {
                            failed_step: step_name,
                            source: Box::new(err),
                            compensation_errors: Vec::new(),
                        });
                    }
                }
            }

            completed.lock().await.push(i);
            ctx.hooks.fire_end(&step_name, ctx);

            if let Some(j) = &ctx.journal {
                let _ = j.record_completed(&ctx.workflow_id, &step_name).await;
            }
        }

        Ok(())
    }

    async fn compensate_completed(&self, ctx: &Context, completed: &[usize]) -> Vec<Error> {
        let timeout = self.compensation_timeout;
        let mut errors = Vec::new();

        // Enter compensation mode so adapter calls don't fail with Cancelled
        ctx.enter_compensation();

        for &i in completed.iter().rev() {
            let step = &self.steps[i];
            let step_name = step.name().to_string();
            ctx.hooks.fire_compensate(&step_name, ctx);

            // Apply a timeout to each compensation step to prevent hangs
            let result = tokio::time::timeout(timeout, step.compensate(ctx)).await;

            match result {
                Ok(Ok(())) => {
                    if let Some(j) = &ctx.journal {
                        let _ = j.record_compensated(&ctx.workflow_id, &step_name).await;
                    }
                }
                Ok(Err(e)) => {
                    errors.push(e);
                }
                Err(_elapsed) => {
                    errors.push(Error::Timeout {
                        step: format!("compensate:{step_name}"),
                        seconds: timeout.as_secs_f64(),
                    });
                }
            }
        }

        ctx.exit_compensation();
        errors
    }
}

#[async_trait]
impl Step for Sequence {
    fn name(&self) -> &str {
        &self.name
    }

    async fn run(&self, ctx: &Context) -> Result<()> {
        let completed = Arc::new(Mutex::new(Vec::<usize>::new()));

        match self.timeout {
            Some(dur) => {
                tokio::select! {
                    result = self.execute(ctx, completed.clone()) => result,
                    _ = tokio::time::sleep(dur) => {
                        // Timeout: compensate everything that completed
                        let comp = completed.lock().await;
                        let comp_errs = self.compensate_completed(ctx, &comp).await;
                        Err(Error::SequenceFailed {
                            failed_step: self.name.clone(),
                            source: Box::new(Error::Timeout {
                                step: self.name.clone(),
                                seconds: dur.as_secs_f64(),
                            }),
                            compensation_errors: comp_errs,
                        })
                    }
                }
            }
            None => self.execute(ctx, completed).await,
        }
    }

    async fn compensate(&self, ctx: &Context) -> Result<()> {
        // Compensate all steps in reverse
        let all: Vec<usize> = (0..self.steps.len()).collect();
        let errors = self.compensate_completed(ctx, &all).await;
        if errors.is_empty() {
            Ok(())
        } else {
            Err(Error::SequenceFailed {
                failed_step: self.name.clone(),
                source: Box::new(Error::TaskFailed {
                    task: self.name.clone(),
                    message: "compensation requested".into(),
                }),
                compensation_errors: errors,
            })
        }
    }
}
