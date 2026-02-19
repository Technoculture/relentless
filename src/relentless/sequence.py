"""Sequence composition with automatic compensation."""

from __future__ import annotations

import asyncio
from typing import Any

from .context import Context
from .errors import SequenceFailed
from .hooks import Hooks
from .result import SequenceResult


async def _call_hook(hook, *args):
    """Call a hook function, handling both sync and async hooks."""
    if hook is None:
        return
    result = hook(*args)
    if asyncio.iscoroutine(result):
        await result


class Sequence:
    """An ordered sequence of steps with automatic compensation on failure.

    Steps can be BoundTasks, Parallels, Guards, Branches, or nested
    Sequences — anything with ``run(ctx)``, ``compensate(ctx)``, and
    ``name``.

    When used inside another Sequence, a Sequence acts as a single
    step: its ``compensate()`` undoes all its completed sub-steps.

    Example::

        inner = rel.Sequence([task_a(), task_b()], name="phase1")
        outer = rel.Sequence([inner, task_c()])
    """

    def __init__(self, tasks: list, *, name: str | None = None):
        self.tasks = list(tasks)
        self._name = name or "sequence"
        self._completed: list = []

    @property
    def name(self) -> str:
        return self._name

    async def run(
        self,
        adapter_or_ctx: Any = None,
        *,
        hooks: Hooks | None = None,
    ) -> SequenceResult:
        """Execute all steps in order. Compensate on failure.

        Can be called two ways:

        * ``seq.run(adapter)`` — top-level call, creates a fresh Context.
        * ``seq.run(ctx)`` — nested call (used when Sequence is a step
          inside another Sequence), reuses the parent Context.

        Args:
            adapter_or_ctx: An Adapter, a Context, or None.
            hooks: Optional lifecycle callbacks.
        """
        if adapter_or_ctx is None:
            from .adapters.local import LocalAdapter

            ctx = Context(adapter=LocalAdapter())
        elif isinstance(adapter_or_ctx, Context):
            ctx = adapter_or_ctx
        else:
            # Assume it's an Adapter
            ctx = Context(adapter=adapter_or_ctx)

        hooks = hooks or Hooks()
        self._completed = []

        for step in self.tasks:
            await _call_hook(hooks.on_step_start, step.name, ctx)
            try:
                await step.run(ctx)
                self._completed.append(step)
                await _call_hook(hooks.on_step_end, step.name, ctx)
            except Exception as e:
                await _call_hook(hooks.on_step_error, step.name, ctx, e)

                # Compensate completed steps in reverse order
                comp_errors: list[Exception] = []
                for done in reversed(self._completed):
                    await _call_hook(hooks.on_compensate, done.name, ctx)
                    try:
                        await done.compensate(ctx)
                    except Exception as comp_err:
                        comp_errors.append(comp_err)

                raise SequenceFailed(
                    failed_task_name=step.name,
                    original_error=e,
                    compensation_errors=comp_errors,
                ) from e

        return SequenceResult(success=True, tasks_completed=len(self._completed))

    async def compensate(self, ctx: Context) -> None:
        """Compensate all steps that completed during the last run.

        Used when this Sequence is nested inside another Sequence.
        """
        for step in reversed(self._completed):
            await step.compensate(ctx)

    def __rshift__(self, other: Any) -> Sequence:
        from .task import BoundTask

        if isinstance(other, (BoundTask, Sequence)):
            return Sequence([self, other])
        if hasattr(other, "run") and hasattr(other, "compensate"):
            return Sequence([self, other])
        return NotImplemented

    def __repr__(self) -> str:
        return f"Sequence({self.tasks!r})"
