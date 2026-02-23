use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::adapter::Adapter;
use crate::cancel::CancellationToken;
use crate::error::{Error, Result};
use crate::hooks::Hooks;
use crate::journal::Journal;
use crate::value::Value;

/// Execution context shared by all steps in a workflow.
///
/// Uses interior mutability so parallel steps can safely share it.
pub struct Context {
    adapter: Arc<dyn Adapter>,
    state: Arc<RwLock<HashMap<String, Value>>>,
    pub workflow_id: String,
    pub cancel: CancellationToken,
    pub hooks: Hooks,
    pub journal: Option<Arc<dyn Journal>>,
    /// When true, cancellation checks are skipped (e.g. during compensation).
    compensating: Arc<AtomicBool>,
}

impl Context {
    pub fn new(adapter: Arc<dyn Adapter>) -> Self {
        Self {
            adapter,
            state: Arc::new(RwLock::new(HashMap::new())),
            workflow_id: gen_id(),
            cancel: CancellationToken::new(),
            hooks: Hooks::default(),
            journal: None,
            compensating: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn with_hooks(mut self, hooks: Hooks) -> Self {
        self.hooks = hooks;
        self
    }

    pub fn with_cancel(mut self, token: CancellationToken) -> Self {
        self.cancel = token;
        self
    }

    pub fn with_journal(mut self, journal: Arc<dyn Journal>) -> Self {
        self.journal = Some(journal);
        self
    }

    // ── adapter I/O ─────────────────────────────────────────────────

    pub async fn execute(&self, action: &str, args: &[Value]) -> Result<Value> {
        self.check_cancelled()?;
        self.adapter.execute(action, args).await
    }

    pub async fn read(&self, key: &str) -> Result<Value> {
        self.check_cancelled()?;
        self.adapter.read(key).await
    }

    pub async fn subscribe(
        &self,
        topic: &str,
    ) -> Result<tokio::sync::mpsc::Receiver<Value>> {
        self.check_cancelled()?;
        self.adapter.subscribe(topic).await
    }

    // ── shared state ────────────────────────────────────────────────

    pub async fn set(&self, key: &str, value: Value) {
        self.state.write().await.insert(key.to_string(), value);
    }

    pub async fn get(&self, key: &str) -> Option<Value> {
        self.state.read().await.get(key).cloned()
    }

    pub async fn get_bool(&self, key: &str) -> bool {
        self.get(key).await.and_then(|v| v.as_bool()).unwrap_or(false)
    }

    pub async fn get_f64(&self, key: &str) -> Option<f64> {
        self.get(key).await.and_then(|v| v.as_f64())
    }

    pub async fn get_str(&self, key: &str) -> Option<String> {
        self.get(key).await.and_then(|v| v.as_str().map(String::from))
    }

    // ── cancellation ────────────────────────────────────────────────

    pub fn check_cancelled(&self) -> Result<()> {
        if self.compensating.load(Ordering::SeqCst) {
            // During compensation, cancellation checks are skipped —
            // compensation MUST be able to run even after cancel.
            return Ok(());
        }
        if self.cancel.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Mark context as being in compensation mode.
    /// During compensation, `check_cancelled()` is a no-op so that
    /// adapter calls can still run to undo completed work.
    pub fn enter_compensation(&self) {
        self.compensating.store(true, Ordering::SeqCst);
    }

    pub fn exit_compensation(&self) {
        self.compensating.store(false, Ordering::SeqCst);
    }
}

fn gen_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{t:x}")
}
