"""Lifecycle hooks for observability."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Callable

from .context import Context


# Hook signature: (step_name: str, ctx: Context, **extra) -> None
# Hooks are sync or async callables. The runner handles both.
HookFn = Callable[..., Any]


@dataclass
class Hooks:
    """Lifecycle callbacks invoked during sequence execution.

    All hooks are optional. Each receives the step name and context.
    Hooks may be sync or async functions.

    Example::

        hooks = Hooks(
            on_step_start=lambda name, ctx: print(f"Starting {name}"),
            on_step_end=lambda name, ctx: print(f"Finished {name}"),
            on_step_error=lambda name, ctx, error: log.error(f"{name}: {error}"),
            on_compensate=lambda name, ctx: print(f"Compensating {name}"),
        )
        await flow.run(adapter, hooks=hooks)
    """

    on_step_start: HookFn | None = None
    on_step_end: HookFn | None = None
    on_step_error: HookFn | None = None
    on_compensate: HookFn | None = None
