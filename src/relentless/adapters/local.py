"""Local (in-process) adapter for testing and simulation."""

import asyncio
from typing import Any, Callable


class LocalAdapter:
    """In-process adapter for testing workflows without external dependencies.

    Register handlers for action names, store values in memory, and
    inspect the call log for assertions.

    Example::

        adapter = LocalAdapter()
        adapter.on("arm.move", lambda dest: print(f"Moving to {dest}"))
        adapter.on("gripper.close", lambda: True)
        result = await flow.run(adapter)
        assert ("arm.move", ("bin_a",), {}) in adapter.calls
    """

    def __init__(self) -> None:
        self.store: dict[str, Any] = {}
        self._handlers: dict[str, Callable] = {}
        self.calls: list[tuple[str, tuple, dict]] = []

    def on(self, action: str, handler: Callable) -> None:
        """Register a handler for an action name."""
        self._handlers[action] = handler

    async def execute(self, action: str, *args: Any, **kwargs: Any) -> Any:
        """Execute a named action.

        Logs the call, then dispatches to a registered handler if one
        exists. Returns None if no handler is registered.
        """
        self.calls.append((action, args, kwargs))
        if action in self._handlers:
            handler = self._handlers[action]
            if asyncio.iscoroutinefunction(handler):
                return await handler(*args, **kwargs)
            return handler(*args, **kwargs)
        return None

    async def read(self, key: str) -> Any:
        """Read a value from the in-memory store."""
        return self.store.get(key)

    def reset(self) -> None:
        """Clear call log and store."""
        self.calls.clear()
        self.store.clear()
