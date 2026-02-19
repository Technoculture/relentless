"""Step protocol — the common interface for composable units."""

from __future__ import annotations

from typing import Any, Protocol, runtime_checkable

from .context import Context


@runtime_checkable
class Step(Protocol):
    """Anything that can run and compensate inside a Sequence.

    BoundTask, Parallel, Guard, Branch, and nested Sequences all
    implement this protocol.
    """

    @property
    def name(self) -> str: ...

    async def run(self, ctx: Context) -> Any: ...

    async def compensate(self, ctx: Context) -> None: ...
