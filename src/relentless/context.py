"""Execution context passed to tasks."""

import uuid
from typing import Any

from .adapter import Adapter


class Context:
    """Execution context available to every task.

    Provides access to the adapter for I/O and a shared state dict
    for passing data between tasks in a sequence.

    Attributes:
        adapter: The I/O adapter for executing actions.
        workflow_id: Unique identifier for this execution run.
        state: Shared dict for inter-task communication.
    """

    def __init__(self, adapter: Adapter, workflow_id: str | None = None):
        self.adapter = adapter
        self.workflow_id = workflow_id or uuid.uuid4().hex[:12]
        self.state: dict[str, Any] = {}

    async def execute(self, action: str, *args: Any, **kwargs: Any) -> Any:
        """Execute an action through the adapter."""
        return await self.adapter.execute(action, *args, **kwargs)

    async def read(self, key: str) -> Any:
        """Read a value through the adapter."""
        return await self.adapter.read(key)
