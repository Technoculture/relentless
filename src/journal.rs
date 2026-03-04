use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::error::Result;

/// Write-ahead log for crash recovery.
///
/// Record which steps completed so that after a restart you can
/// skip already-done work or compensate what was left hanging.
#[async_trait]
pub trait Journal: Send + Sync {
    async fn record_started(&self, workflow_id: &str, step: &str) -> Result<()>;
    async fn record_completed(&self, workflow_id: &str, step: &str) -> Result<()>;
    async fn record_failed(&self, workflow_id: &str, step: &str, error: &str) -> Result<()>;
    async fn record_compensated(&self, workflow_id: &str, step: &str) -> Result<()>;
    async fn completed_steps(&self, workflow_id: &str) -> Result<Vec<String>>;
}

/// In-memory journal for testing.
pub struct MemoryJournal {
    entries: Mutex<Vec<JournalEntry>>,
}

#[derive(Debug, Clone)]
pub struct JournalEntry {
    pub workflow_id: String,
    pub step: String,
    pub kind: EntryKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EntryKind {
    Started,
    Completed,
    Failed(String),
    Compensated,
}

impl MemoryJournal {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            entries: Mutex::new(Vec::new()),
        })
    }

    pub async fn entries(&self) -> Vec<JournalEntry> {
        self.entries.lock().await.clone()
    }
}

#[async_trait]
impl Journal for MemoryJournal {
    async fn record_started(&self, workflow_id: &str, step: &str) -> Result<()> {
        self.entries.lock().await.push(JournalEntry {
            workflow_id: workflow_id.into(),
            step: step.into(),
            kind: EntryKind::Started,
        });
        Ok(())
    }

    async fn record_completed(&self, workflow_id: &str, step: &str) -> Result<()> {
        self.entries.lock().await.push(JournalEntry {
            workflow_id: workflow_id.into(),
            step: step.into(),
            kind: EntryKind::Completed,
        });
        Ok(())
    }

    async fn record_failed(&self, workflow_id: &str, step: &str, error: &str) -> Result<()> {
        self.entries.lock().await.push(JournalEntry {
            workflow_id: workflow_id.into(),
            step: step.into(),
            kind: EntryKind::Failed(error.into()),
        });
        Ok(())
    }

    async fn record_compensated(&self, workflow_id: &str, step: &str) -> Result<()> {
        self.entries.lock().await.push(JournalEntry {
            workflow_id: workflow_id.into(),
            step: step.into(),
            kind: EntryKind::Compensated,
        });
        Ok(())
    }

    async fn completed_steps(&self, workflow_id: &str) -> Result<Vec<String>> {
        let entries = self.entries.lock().await;
        Ok(entries
            .iter()
            .filter(|e| e.workflow_id == workflow_id && e.kind == EntryKind::Completed)
            .map(|e| e.step.clone())
            .collect())
    }
}
