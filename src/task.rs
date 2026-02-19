use async_trait::async_trait;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::context::Context;
use crate::error::Result;
use crate::step::{ErrorStrategy, Step};

type BoxFut<'a> = Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;
type RunFn = Arc<dyn Fn(&Context) -> BoxFut<'_> + Send + Sync>;

/// A step built from closures. Ergonomic alternative to implementing `Step`.
///
/// ```ignore
/// let pick = FnTask::new("pick_part", |ctx| Box::pin(async move {
///     ctx.execute("gripper.close", &[]).await?;
///     Ok(())
/// }));
/// ```
pub struct FnTask {
    name: String,
    run_fn: RunFn,
    compensate_fn: Option<RunFn>,
}

impl FnTask {
    pub fn new<F>(name: impl Into<String>, f: F) -> Self
    where
        F: Fn(&Context) -> BoxFut<'_> + Send + Sync + 'static,
    {
        Self {
            name: name.into(),
            run_fn: Arc::new(f),
            compensate_fn: None,
        }
    }

    pub fn with_compensate<F>(mut self, f: F) -> Self
    where
        F: Fn(&Context) -> BoxFut<'_> + Send + Sync + 'static,
    {
        self.compensate_fn = Some(Arc::new(f));
        self
    }
}

#[async_trait]
impl Step for FnTask {
    fn name(&self) -> &str {
        &self.name
    }

    async fn run(&self, ctx: &Context) -> Result<()> {
        (self.run_fn)(ctx).await
    }

    async fn compensate(&self, ctx: &Context) -> Result<()> {
        if let Some(f) = &self.compensate_fn {
            f(ctx).await
        } else {
            Ok(())
        }
    }

    fn error_strategy(&self, _error: &crate::error::Error) -> ErrorStrategy {
        ErrorStrategy::Compensate
    }
}
