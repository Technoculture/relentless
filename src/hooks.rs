use std::sync::Arc;

use crate::context::Context;
use crate::error::Error;

type HookFn = Arc<dyn Fn(&str, &Context) + Send + Sync>;
type ErrorHookFn = Arc<dyn Fn(&str, &Context, &Error) + Send + Sync>;

/// Lifecycle callbacks invoked during sequence execution.
///
/// All hooks are synchronous and non-blocking.  For async logging,
/// send to a channel inside the hook.
#[derive(Clone, Default)]
pub struct Hooks {
    pub on_step_start: Option<HookFn>,
    pub on_step_end: Option<HookFn>,
    pub on_step_error: Option<ErrorHookFn>,
    pub on_compensate: Option<HookFn>,
}

impl Hooks {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn on_step_start(mut self, f: impl Fn(&str, &Context) + Send + Sync + 'static) -> Self {
        self.on_step_start = Some(Arc::new(f));
        self
    }

    pub fn on_step_end(mut self, f: impl Fn(&str, &Context) + Send + Sync + 'static) -> Self {
        self.on_step_end = Some(Arc::new(f));
        self
    }

    pub fn on_step_error(
        mut self,
        f: impl Fn(&str, &Context, &Error) + Send + Sync + 'static,
    ) -> Self {
        self.on_step_error = Some(Arc::new(f));
        self
    }

    pub fn on_compensate(mut self, f: impl Fn(&str, &Context) + Send + Sync + 'static) -> Self {
        self.on_compensate = Some(Arc::new(f));
        self
    }

    pub(crate) fn fire_start(&self, name: &str, ctx: &Context) {
        if let Some(f) = &self.on_step_start {
            f(name, ctx);
        }
    }

    pub(crate) fn fire_end(&self, name: &str, ctx: &Context) {
        if let Some(f) = &self.on_step_end {
            f(name, ctx);
        }
    }

    pub(crate) fn fire_error(&self, name: &str, ctx: &Context, err: &Error) {
        if let Some(f) = &self.on_step_error {
            f(name, ctx, err);
        }
    }

    pub(crate) fn fire_compensate(&self, name: &str, ctx: &Context) {
        if let Some(f) = &self.on_compensate {
            f(name, ctx);
        }
    }
}
