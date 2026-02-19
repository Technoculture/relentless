use async_trait::async_trait;
use std::time::Duration;

use crate::context::Context;
use crate::error::{Error, Result};
use crate::retry::RetryPolicy;
use crate::step::{ErrorStrategy, Step};

/// Linear execution with automatic reverse-order compensation on failure.
///
/// Supports: retry, timeout, cancellation, hooks, journaling,
/// and per-step error discrimination (Compensate / Skip / Escalate).
pub struct Sequence {
    name: String,
    steps: Vec<Box<dyn Step>>,
    retry: RetryPolicy,
    timeout: Option<Duration>,
}

impl Sequence {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            steps: Vec::new(),
            retry: RetryPolicy::none(),
            timeout: None,
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

    /// Execute the sequence, returning the index of the first completed step
    /// on both success and failure paths.
    async fn execute(&self, ctx: &Context) -> Result<()> {
        let mut completed: Vec<usize> = Vec::new();

        for (i, step) in self.steps.iter().enumerate() {
            ctx.check_cancelled()?;

            let step_name = step.name().to_string();
            ctx.hooks.fire_start(&step_name, ctx);

            // Journal: record started
            if let Some(j) = &ctx.journal {
                let _ = j.record_started(&ctx.workflow_id, &step_name).await;
            }

            // Skip already-completed steps (crash recovery)
            if let Some(j) = &ctx.journal {
                let done = j.completed_steps(&ctx.workflow_id).await.unwrap_or_default();
                if done.contains(&step_name) {
                    completed.push(i);
                    continue;
                }
            }

            // Run with retry
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
                        last_err = Some(e);
                    }
                }
            }

            if let Some(err) = last_err {
                ctx.hooks.fire_error(&step_name, ctx, &err);

                if let Some(j) = &ctx.journal {
                    let _ = j
                        .record_failed(&ctx.workflow_id, &step_name, &err.to_string())
                        .await;
                }

                match step.error_strategy(&err) {
                    ErrorStrategy::Compensate => {
                        let comp_errs = self.compensate_completed(ctx, &completed).await;
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

            completed.push(i);
            ctx.hooks.fire_end(&step_name, ctx);

            if let Some(j) = &ctx.journal {
                let _ = j.record_completed(&ctx.workflow_id, &step_name).await;
            }
        }

        Ok(())
    }

    async fn compensate_completed(&self, ctx: &Context, completed: &[usize]) -> Vec<Error> {
        let mut errors = Vec::new();
        for &i in completed.iter().rev() {
            let step = &self.steps[i];
            let step_name = step.name().to_string();
            ctx.hooks.fire_compensate(&step_name, ctx);

            if let Err(e) = step.compensate(ctx).await {
                errors.push(e);
            } else if let Some(j) = &ctx.journal {
                let _ = j.record_compensated(&ctx.workflow_id, &step_name).await;
            }
        }
        errors
    }
}

#[async_trait]
impl Step for Sequence {
    fn name(&self) -> &str {
        &self.name
    }

    async fn run(&self, ctx: &Context) -> Result<()> {
        match self.timeout {
            Some(dur) => {
                tokio::select! {
                    result = self.execute(ctx) => result,
                    _ = tokio::time::sleep(dur) => {
                        Err(Error::Timeout {
                            step: self.name.clone(),
                            seconds: dur.as_secs_f64(),
                        })
                    }
                }
            }
            None => self.execute(ctx).await,
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
