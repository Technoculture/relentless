use async_trait::async_trait;

use crate::context::Context;
use crate::error::{Error, Result};

/// How the runner should react when this step fails.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ErrorStrategy {
    /// Compensate all completed steps in reverse (default).
    Compensate,
    /// Skip the failed step and continue.
    Skip,
    /// Stop immediately, do NOT compensate.
    Escalate,
}

/// The core abstraction: anything that can run and compensate.
///
/// Implement this for your robot operations.  `Sequence`, `Parallel`,
/// `Guard`, `Branch`, and `Loop` also implement `Step`, so they nest
/// freely.
#[async_trait]
pub trait Step: Send + Sync {
    fn name(&self) -> &str;

    async fn run(&self, ctx: &Context) -> Result<()>;

    /// Undo or mitigate the effects of `run`.
    /// Default is a no-op (step has no side-effects to undo).
    async fn compensate(&self, ctx: &Context) -> Result<()> {
        let _ = ctx;
        Ok(())
    }

    /// Override to customise per-error behaviour.
    fn error_strategy(&self, _error: &Error) -> ErrorStrategy {
        ErrorStrategy::Compensate
    }
}
