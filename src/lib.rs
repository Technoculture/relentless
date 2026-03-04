//! # Relentless
//!
//! Compensatable task sequences for robotics.
//!
//! Build workflows as composable `Step` trees with automatic reverse-order
//! compensation (saga pattern), retry, timeout, cancellation, parallel
//! execution, resource locking, journaling, and sensor streaming.
//!
//! ## Quick start
//!
//! ```ignore
//! use relentless::*;
//! use std::sync::Arc;
//!
//! let adapter = LocalAdapter::new();
//! let ctx = Context::new(adapter);
//!
//! let workflow = Sequence::new("pick_and_place")
//!     .step(FnTask::new("pick", |ctx| Box::pin(async move {
//!         ctx.execute("gripper.close", &[]).await?;
//!         Ok(())
//!     })))
//!     .step(FnTask::new("place", |ctx| Box::pin(async move {
//!         ctx.execute("gripper.open", &[]).await?;
//!         Ok(())
//!     })));
//!
//! workflow.run(&ctx).await.unwrap();
//! ```

mod adapter;
mod cancel;
mod combinators;
mod context;
mod error;
mod hooks;
mod journal;
mod local_adapter;
mod parallel;
mod resource;
mod retry;
mod sequence;
mod step;
mod task;
mod value;

pub use adapter::Adapter;
pub use cancel::CancellationToken;
pub use combinators::{Branch, Guard, Loop};
pub use context::Context;
pub use error::{Error, Result};
pub use hooks::Hooks;
pub use journal::{EntryKind, Journal, JournalEntry, MemoryJournal};
pub use local_adapter::LocalAdapter;
pub use parallel::Parallel;
pub use resource::{Locked, ResourceLock};
pub use retry::{Backoff, RetryPolicy};
pub use sequence::Sequence;
pub use step::{ErrorStrategy, Step};
pub use task::FnTask;
pub use value::Value;
