"""Combinators: guard, branch — control-flow steps for sequences."""

from __future__ import annotations

from typing import Any, Callable

from .context import Context
from .errors import GuardFailed


class Guard:
    """Run a step only if a condition holds, else skip or fail.

    Example::

        rel.Guard(
            lambda ctx: ctx.state.get("force", 0) < 5.0,
            insert_peg(),
        )

    Args:
        condition: Sync or async callable ``(ctx) -> bool``.
        step: Step to run if condition is truthy.
        otherwise: Optional step to run if condition is falsy.
            If None and condition is falsy, raises GuardFailed.
    """

    def __init__(
        self,
        condition: Callable,
        step: Any,
        *,
        otherwise: Any | None = None,
        name: str | None = None,
    ):
        self._condition = condition
        self._step = step
        self._otherwise = otherwise
        self._name = name or f"guard({step.name})"
        self._ran: Any | None = None  # track which step ran for compensation

    @property
    def name(self) -> str:
        return self._name

    async def run(self, ctx: Context) -> Any:
        import asyncio

        cond = self._condition(ctx)
        if asyncio.iscoroutine(cond):
            cond = await cond

        if cond:
            self._ran = self._step
            return await self._step.run(ctx)
        elif self._otherwise is not None:
            self._ran = self._otherwise
            return await self._otherwise.run(ctx)
        else:
            self._ran = None
            raise GuardFailed(f"Guard condition failed for {self._step.name}")

    async def compensate(self, ctx: Context) -> None:
        if self._ran is not None:
            await self._ran.compensate(ctx)


class Branch:
    """Route execution to one of several steps based on a selector.

    Example::

        rel.Branch(
            lambda ctx: ctx.state["defect_type"],
            {
                "none":       route_to_packaging(),
                "cosmetic":   route_to_rework(),
                "structural": route_to_reject(),
            },
            default=stop_and_call_operator(),
        )

    Args:
        selector: Sync or async callable ``(ctx) -> key``.
        branches: Dict mapping keys to steps.
        default: Step to run if selector returns a key not in branches.
            If None and key is missing, raises KeyError.
    """

    def __init__(
        self,
        selector: Callable,
        branches: dict[Any, Any],
        *,
        default: Any | None = None,
        name: str | None = None,
    ):
        self._selector = selector
        self._branches = dict(branches)
        self._default = default
        self._name = name or "branch"
        self._ran: Any | None = None

    @property
    def name(self) -> str:
        return self._name

    async def run(self, ctx: Context) -> Any:
        import asyncio

        key = self._selector(ctx)
        if asyncio.iscoroutine(key):
            key = await key

        step = self._branches.get(key, self._default)
        if step is None:
            raise KeyError(
                f"Branch selector returned {key!r}, "
                f"not in {list(self._branches.keys())} and no default set"
            )

        self._ran = step
        return await step.run(ctx)

    async def compensate(self, ctx: Context) -> None:
        if self._ran is not None:
            await self._ran.compensate(ctx)
