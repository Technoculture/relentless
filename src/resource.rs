use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::context::Context;
use crate::error::{Error, Result};
use crate::step::{ErrorStrategy, Step};

/// Wraps a step with a shared mutex for resource coordination.
///
/// Use when multiple workflows or parallel branches access the same
/// physical resource (shared pallet zone, single gripper, etc).
///
/// ```ignore
/// let pallet_zone = ResourceLock::new();
/// let step_a = Locked::new(PlaceOnPallet("A1"), pallet_zone.clone());
/// let step_b = Locked::new(PlaceOnPallet("A2"), pallet_zone);
/// ```
pub struct Locked {
    inner: Box<dyn Step>,
    lock: ResourceLock,
}

/// A named mutex that can be shared across steps.
#[derive(Clone)]
pub struct ResourceLock {
    name: String,
    mu: Arc<Mutex<()>>,
}

impl ResourceLock {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            mu: Arc::new(Mutex::new(())),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

impl Locked {
    pub fn new(step: impl Step + 'static, lock: ResourceLock) -> Self {
        Self {
            inner: Box::new(step),
            lock,
        }
    }
}

#[async_trait]
impl Step for Locked {
    fn name(&self) -> &str {
        self.inner.name()
    }

    async fn run(&self, ctx: &Context) -> Result<()> {
        let _guard = self.lock.mu.lock().await;
        self.inner.run(ctx).await
    }

    async fn compensate(&self, ctx: &Context) -> Result<()> {
        let _guard = self.lock.mu.lock().await;
        self.inner.compensate(ctx).await
    }

    fn error_strategy(&self, error: &Error) -> ErrorStrategy {
        self.inner.error_strategy(error)
    }
}
