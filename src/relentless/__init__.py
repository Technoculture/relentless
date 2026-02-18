"""Relentless: Compensatable task sequences for robotics.

A lightweight library for composing async tasks with automatic
compensation (undo) on failure. Transport-agnostic, testable by default.
"""

from .adapter import Adapter
from .context import Context
from .errors import CompensationFailed, SequenceFailed, TaskFailed
from .result import SequenceResult
from .retry import RetryPolicy
from .sequence import Sequence
from .task import BoundTask, Task, task

__all__ = [
    "task",
    "Task",
    "BoundTask",
    "Sequence",
    "Context",
    "Adapter",
    "RetryPolicy",
    "SequenceResult",
    "TaskFailed",
    "SequenceFailed",
    "CompensationFailed",
]
