"""Adapter protocol for I/O abstraction."""

from typing import Any, Protocol, runtime_checkable


@runtime_checkable
class Adapter(Protocol):
    """Protocol that adapters must implement.

    Adapters bridge relentless tasks to the outside world.
    Different adapters handle different transports
    (local, Zenoh, ROS2, MQTT, etc).
    """

    async def execute(self, action: str, *args: Any, **kwargs: Any) -> Any:
        """Execute a named action."""
        ...

    async def read(self, key: str) -> Any:
        """Read a value by key."""
        ...
