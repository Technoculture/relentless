use std::fmt;

/// All errors produced by relentless.
#[derive(Debug)]
pub enum Error {
    /// A task failed after exhausting retries.
    TaskFailed {
        task: String,
        message: String,
    },
    /// A sequence failed; includes compensation status.
    SequenceFailed {
        failed_step: String,
        source: Box<Error>,
        compensation_errors: Vec<Error>,
    },
    /// A parallel step failed.
    ParallelFailed {
        failed_step: String,
        source: Box<Error>,
        compensation_errors: Vec<Error>,
    },
    /// A guard condition was false with no fallback.
    GuardFailed {
        step: String,
    },
    /// Operation timed out.
    Timeout {
        step: String,
        seconds: f64,
    },
    /// Cancelled via CancellationToken.
    Cancelled,
    /// Operation not supported by adapter.
    Unsupported {
        operation: String,
    },
    /// Arbitrary external error.
    External(Box<dyn std::error::Error + Send + Sync>),
}

impl Error {
    pub fn task_failed(task: &str, message: &str) -> Self {
        Error::TaskFailed {
            task: task.to_string(),
            message: message.to_string(),
        }
    }

    pub fn external(err: impl std::error::Error + Send + Sync + 'static) -> Self {
        Error::External(Box::new(err))
    }

    pub fn is_cancelled(&self) -> bool {
        matches!(self, Error::Cancelled)
    }

    /// True if all compensations succeeded (for SequenceFailed/ParallelFailed).
    pub fn fully_compensated(&self) -> bool {
        match self {
            Error::SequenceFailed { compensation_errors, .. } => compensation_errors.is_empty(),
            Error::ParallelFailed { compensation_errors, .. } => compensation_errors.is_empty(),
            _ => true,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::TaskFailed { task, message } => {
                write!(f, "task '{task}' failed: {message}")
            }
            Error::SequenceFailed { failed_step, source, compensation_errors } => {
                write!(f, "sequence failed at '{failed_step}': {source}")?;
                if !compensation_errors.is_empty() {
                    write!(f, " ({} compensation errors)", compensation_errors.len())?;
                }
                Ok(())
            }
            Error::ParallelFailed { failed_step, source, compensation_errors } => {
                write!(f, "parallel failed at '{failed_step}': {source}")?;
                if !compensation_errors.is_empty() {
                    write!(f, " ({} compensation errors)", compensation_errors.len())?;
                }
                Ok(())
            }
            Error::GuardFailed { step } => {
                write!(f, "guard failed for '{step}'")
            }
            Error::Timeout { step, seconds } => {
                write!(f, "'{step}' timed out after {seconds}s")
            }
            Error::Cancelled => write!(f, "cancelled"),
            Error::Unsupported { operation } => {
                write!(f, "unsupported: {operation}")
            }
            Error::External(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::External(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
