"""Task definition and composition."""

from __future__ import annotations

import asyncio
import functools
from typing import Any, Callable

from .context import Context
from .errors import TaskFailed
from .retry import DEFAULT_RETRY, RetryPolicy


class Task:
    """A decorated async function that can be composed and compensated.

    Do not instantiate directly -- use the ``@task`` decorator.
    """

    def __init__(
        self,
        fn: Callable,
        *,
        retry_policy: RetryPolicy = DEFAULT_RETRY,
        timeout: float | None = None,
        name: str | None = None,
    ):
        self.fn = fn
        self.undo_fn: Callable | None = None
        self.retry_policy = retry_policy
        self.timeout = timeout
        self.name = name or fn.__name__
        functools.update_wrapper(self, fn)

    def __call__(self, *args: Any, **kwargs: Any) -> BoundTask:
        """Bind arguments, returning a BoundTask for composition."""
        return BoundTask(task=self, args=args, kwargs=kwargs)

    def undo(self, fn: Callable) -> Callable:
        """Register a compensation function for this task.

        Usage::

            @my_task.undo
            async def _(ctx, ...):
                # undo logic
        """
        self.undo_fn = fn
        return fn


class BoundTask:
    """A task with arguments bound, ready for execution or composition."""

    def __init__(self, task: Task, args: tuple, kwargs: dict):
        self.task = task
        self.args = args
        self.kwargs = kwargs

    @property
    def name(self) -> str:
        return self.task.name

    async def run(self, ctx: Context) -> Any:
        """Execute this task with retry and timeout policy."""
        last_error: Exception | None = None
        policy = self.task.retry_policy

        for attempt in range(1, policy.max_attempts + 1):
            delay = policy.delay_for_attempt(attempt)
            if delay > 0:
                await asyncio.sleep(delay)

            try:
                coro = self.task.fn(ctx, *self.args, **self.kwargs)
                if self.task.timeout:
                    return await asyncio.wait_for(coro, timeout=self.task.timeout)
                return await coro
            except asyncio.TimeoutError:
                last_error = TimeoutError(
                    f"Task '{self.name}' timed out after {self.task.timeout}s"
                )
            except Exception as e:
                last_error = e

        raise TaskFailed(
            f"Task '{self.name}' failed after {policy.max_attempts} attempt(s): "
            f"{last_error}"
        ) from last_error

    async def compensate(self, ctx: Context) -> None:
        """Run compensation for this task, if registered."""
        if self.task.undo_fn is not None:
            await self.task.undo_fn(ctx, *self.args, **self.kwargs)

    def __rshift__(self, other: Any) -> Any:
        """Compose tasks with the ``>>`` operator."""
        from .sequence import Sequence

        if isinstance(other, BoundTask):
            return Sequence([self, other])
        if isinstance(other, Sequence):
            return Sequence([self] + other.tasks)
        return NotImplemented

    def __repr__(self) -> str:
        parts = [repr(a) for a in self.args]
        parts += [f"{k}={v!r}" for k, v in self.kwargs.items()]
        return f"BoundTask({self.name}({', '.join(parts)}))"


def task(
    fn: Callable | None = None,
    *,
    retry: int = 1,
    backoff: str = "none",
    base_delay: float = 1.0,
    max_delay: float = 60.0,
    jitter: bool = False,
    timeout: float | None = None,
    name: str | None = None,
) -> Task | Callable[..., Task]:
    """Decorator to create a Task from an async function.

    Can be used with or without arguments::

        @task
        async def simple(ctx):
            ...

        @task(retry=3, backoff="exponential", timeout=5.0)
        async def resilient(ctx):
            ...
    """
    retry_policy = RetryPolicy(
        max_attempts=retry,
        backoff=backoff,
        base_delay=base_delay,
        max_delay=max_delay,
        jitter=jitter,
    )

    def decorator(fn: Callable) -> Task:
        return Task(fn, retry_policy=retry_policy, timeout=timeout, name=name)

    if fn is not None:
        # Used as @task without arguments
        return Task(fn, retry_policy=DEFAULT_RETRY, timeout=None, name=None)

    return decorator
