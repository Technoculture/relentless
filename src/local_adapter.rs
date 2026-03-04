use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

use crate::adapter::Adapter;
use crate::error::{Error, Result};
use crate::value::Value;

/// In-memory adapter for testing and simulation.
///
/// Pre-load values with `set()`, capture executed actions with `actions()`.
///
/// ```ignore
/// let adapter = LocalAdapter::new();
/// adapter.set("sensor.force", Value::F64(12.5)).await;
/// let ctx = Context::new(adapter);
/// ```
pub struct LocalAdapter {
    values: Mutex<HashMap<String, Value>>,
    actions: Mutex<Vec<(String, Vec<Value>)>>,
    subscribers: Mutex<HashMap<String, Vec<mpsc::Sender<Value>>>>,
}

impl LocalAdapter {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            values: Mutex::new(HashMap::new()),
            actions: Mutex::new(Vec::new()),
            subscribers: Mutex::new(HashMap::new()),
        })
    }

    /// Pre-load a value for `read()`.
    pub async fn set(&self, key: &str, value: Value) {
        self.values.lock().await.insert(key.to_string(), value);
    }

    /// Get all executed actions (name + args).
    pub async fn actions(&self) -> Vec<(String, Vec<Value>)> {
        self.actions.lock().await.clone()
    }

    /// Publish a value to all subscribers on a topic.
    pub async fn publish(&self, topic: &str, value: Value) {
        let subs = self.subscribers.lock().await;
        if let Some(senders) = subs.get(topic) {
            for tx in senders {
                let _ = tx.send(value.clone()).await;
            }
        }
    }
}

#[async_trait]
impl Adapter for LocalAdapter {
    async fn execute(&self, action: &str, args: &[Value]) -> Result<Value> {
        self.actions
            .lock()
            .await
            .push((action.to_string(), args.to_vec()));
        Ok(Value::None)
    }

    async fn read(&self, key: &str) -> Result<Value> {
        let values = self.values.lock().await;
        values.get(key).cloned().ok_or_else(|| Error::TaskFailed {
            task: "read".to_string(),
            message: format!("key '{key}' not found"),
        })
    }

    async fn subscribe(&self, topic: &str) -> Result<mpsc::Receiver<Value>> {
        let (tx, rx) = mpsc::channel(64);
        self.subscribers
            .lock()
            .await
            .entry(topic.to_string())
            .or_default()
            .push(tx);
        Ok(rx)
    }
}
