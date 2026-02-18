"""Result types for sequence execution."""

from dataclasses import dataclass, field


@dataclass
class SequenceResult:
    """Result of running a sequence of tasks.

    Attributes:
        success: True if all tasks completed without error.
        tasks_completed: Number of tasks that ran successfully.
    """

    success: bool
    tasks_completed: int = 0
