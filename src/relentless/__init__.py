"""Relentless: Compensatable task sequences for robotics.

A lightweight library for composing async tasks with automatic
compensation (undo) on failure. Transport-agnostic, testable by default.
"""

from .adapter import Adapter
from .combinators import Branch, Guard
from .context import Context
from .errors import (
    CompensationFailed,
    GuardFailed,
    ParallelFailed,
    SequenceFailed,
    TaskFailed,
)
from .hooks import Hooks
from .parallel import Parallel
from .result import SequenceResult
from .retry import RetryPolicy
from .sequence import Sequence
from .task import BoundTask, Task, task

__all__ = [
    # Core
    "task",
    "Task",
    "BoundTask",
    "Sequence",
    "Parallel",
    # Combinators
    "Guard",
    "Branch",
    # Infrastructure
    "Context",
    "Adapter",
    "Hooks",
    "RetryPolicy",
    "SequenceResult",
    # Errors
    "TaskFailed",
    "SequenceFailed",
    "ParallelFailed",
    "CompensationFailed",
    "GuardFailed",
]
