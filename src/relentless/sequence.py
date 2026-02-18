"""Sequence composition with automatic compensation."""

from __future__ import annotations

from typing import TYPE_CHECKING, Any

from .context import Context
from .errors import SequenceFailed
from .result import SequenceResult

if TYPE_CHECKING:
    from .adapter import Adapter
    from .task import BoundTask


class Sequence:
    """An ordered sequence of tasks with automatic compensation on failure.

    When a task fails (after exhausting its retries), all previously
    completed tasks are compensated in reverse order.
    """

    def __init__(self, tasks: list[BoundTask]):
        self.tasks = list(tasks)

    async def run(self, adapter: Adapter | None = None) -> SequenceResult:
        """Execute all tasks in order. Compensate on failure.

        Args:
            adapter: I/O adapter. If None, uses a no-op LocalAdapter.

        Returns:
            SequenceResult on success.

        Raises:
            SequenceFailed: If a task fails, after compensating completed tasks.
        """
        if adapter is None:
            from .adapters.local import LocalAdapter

            adapter = LocalAdapter()

        ctx = Context(adapter=adapter)
        completed: list[BoundTask] = []

        for bound_task in self.tasks:
            try:
                await bound_task.run(ctx)
                completed.append(bound_task)
            except Exception as e:
                # Compensate completed tasks in reverse order
                comp_errors: list[Exception] = []
                for done in reversed(completed):
                    try:
                        await done.compensate(ctx)
                    except Exception as comp_err:
                        comp_errors.append(comp_err)

                raise SequenceFailed(
                    failed_task_name=bound_task.name,
                    original_error=e,
                    compensation_errors=comp_errors,
                ) from e

        return SequenceResult(success=True, tasks_completed=len(completed))

    def __rshift__(self, other: Any) -> Sequence:
        """Compose sequences with the ``>>`` operator."""
        from .task import BoundTask

        if isinstance(other, BoundTask):
            return Sequence(self.tasks + [other])
        if isinstance(other, Sequence):
            return Sequence(self.tasks + other.tasks)
        return NotImplemented

    def __repr__(self) -> str:
        return f"Sequence({self.tasks!r})"
