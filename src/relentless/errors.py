"""Relentless error types."""


class TaskFailed(Exception):
    """Raised when a task fails after exhausting retries."""

    pass


class CompensationFailed(Exception):
    """Raised when compensation itself fails."""

    def __init__(self, original_error, compensation_errors):
        self.original_error = original_error
        self.compensation_errors = compensation_errors
        super().__init__(
            f"Compensation failed: {original_error} "
            f"(compensation errors: {compensation_errors})"
        )


class SequenceFailed(Exception):
    """Raised when a sequence fails at a task.

    Attributes:
        failed_task_name: Name of the task that failed.
        original_error: The exception that caused the failure.
        compensation_errors: List of exceptions from compensation attempts.
        compensated: True if all compensations succeeded.
    """

    def __init__(self, failed_task_name, original_error, compensation_errors=None):
        self.failed_task_name = failed_task_name
        self.original_error = original_error
        self.compensation_errors = compensation_errors or []
        self.compensated = len(self.compensation_errors) == 0
        msg = f"Task '{failed_task_name}' failed: {original_error}"
        if self.compensation_errors:
            msg += f" (compensation also failed: {self.compensation_errors})"
        super().__init__(msg)
