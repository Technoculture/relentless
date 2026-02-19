use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::error::{Error, Result};
use crate::value::Value;

/// I/O bridge between tasks and the outside world.
///
/// Implement this for your transport: Zenoh, ROS 2, MQTT, gRPC, or
/// direct hardware calls. Use [`LocalAdapter`](crate::LocalAdapter)
/// for testing.
#[async_trait]
pub trait Adapter: Send + Sync {
    /// Execute a named action with arguments.
    async fn execute(&self, action: &str, args: &[Value]) -> Result<Value>;

    /// Read a named value (sensor, config, etc).
    async fn read(&self, key: &str) -> Result<Value>;

    /// Subscribe to a stream of values on a topic.
    ///
    /// Default returns `Unsupported`. Override for adapters that
    /// support streaming (Zenoh, ROS 2 topics, MQTT, etc).
    async fn subscribe(&self, topic: &str) -> Result<mpsc::Receiver<Value>> {
        Err(Error::Unsupported {
            operation: format!("subscribe({topic})"),
        })
    }
}
