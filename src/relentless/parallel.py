"""Parallel execution of steps with coordinated compensation."""

from __future__ import annotations

import asyncio
from typing import Any

from .context import Context
from .errors import ParallelFailed


class Parallel:
    """Run multiple steps concurrently.

    On success, all steps complete. On failure of any step:
      1. Cancel still-running siblings.
      2. Compensate all completed siblings.
      3. Raise ParallelFailed.

    When a Parallel is used inside a Sequence and the Sequence later
    fails, the Parallel's ``compensate()`` undoes all its steps.

    Example::

        flow = rel.Sequence([
            rel.Parallel([
                move_arm("left", "hold"),
                move_arm("right", "approach"),
            ]),
            close_gripper("left"),
        ])
    """

    def __init__(self, steps: list, *, name: str | None = None):
        self.steps = list(steps)
        self._name = name or "parallel"
        self._completed: list = []

    @property
    def name(self) -> str:
        return self._name

    async def run(self, ctx: Context) -> list[Any]:
        """Run all steps concurrently. Returns list of results."""
        self._completed = []

        async def _run_one(step, index):
            result = await step.run(ctx)
            return (index, step, result)

        tasks = {
            asyncio.ensure_future(_run_one(s, i)): (i, s)
            for i, s in enumerate(self.steps)
        }

        results: dict[int, Any] = {}
        first_error: Exception | None = None
        failed_name: str | None = None

        pending = set(tasks.keys())
        while pending:
            done, pending = await asyncio.wait(
                pending, return_when=asyncio.FIRST_COMPLETED
            )
            for fut in done:
                if fut.cancelled():
                    continue
                exc = fut.exception()
                if exc is not None:
                    if first_error is None:
                        first_error = exc
                        failed_name = tasks[fut][1].name
                        # Cancel remaining
                        for p in pending:
                            p.cancel()
                else:
                    idx, step, result = fut.result()
                    self._completed.append(step)
                    results[idx] = result

        if first_error is not None:
            # Compensate completed steps (reverse completion order)
            comp_errors: list[Exception] = []
            for step in reversed(self._completed):
                try:
                    await step.compensate(ctx)
                except Exception as e:
                    comp_errors.append(e)

            raise ParallelFailed(
                failed_step_name=failed_name or "unknown",
                original_error=first_error,
                compensation_errors=comp_errors,
            ) from first_error

        # All succeeded — order results by original index
        return [results[i] for i in range(len(self.steps))]

    async def compensate(self, ctx: Context) -> None:
        """Compensate all steps that completed during the last run."""
        for step in reversed(self._completed):
            await step.compensate(ctx)

    def __rshift__(self, other: Any) -> Any:
        from .sequence import Sequence
        from .task import BoundTask

        if isinstance(other, (BoundTask, Parallel)):
            return Sequence([self, other])
        if isinstance(other, Sequence):
            return Sequence([self] + other.tasks)
        return NotImplemented

    def __repr__(self) -> str:
        return f"Parallel({self.steps!r})"
