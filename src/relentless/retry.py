"""Retry policies with configurable backoff."""

import random
from dataclasses import dataclass


def _fibonacci(n: int) -> int:
    """Return the nth Fibonacci number (0-indexed)."""
    a, b = 0, 1
    for _ in range(n):
        a, b = b, a + b
    return a


@dataclass(frozen=True)
class RetryPolicy:
    """Configurable retry policy with backoff.

    Args:
        max_attempts: Maximum number of execution attempts (1 = no retry).
        backoff: Backoff strategy ("none", "linear", "exponential", "fibonacci").
        base_delay: Base delay in seconds for backoff calculation.
        max_delay: Maximum delay cap in seconds.
        jitter: If True, randomize delay by ±50%.
    """

    max_attempts: int = 1
    backoff: str = "none"
    base_delay: float = 1.0
    max_delay: float = 60.0
    jitter: bool = False

    def delay_for_attempt(self, attempt: int) -> float:
        """Calculate delay before the given attempt number (1-indexed).

        Attempt 1 always returns 0 (no delay before first try).
        """
        if attempt <= 1 or self.backoff == "none":
            return 0.0

        n = attempt - 1  # 0-indexed retry count

        if self.backoff == "linear":
            delay = self.base_delay * n
        elif self.backoff == "exponential":
            delay = self.base_delay * (2 ** (n - 1))
        elif self.backoff == "fibonacci":
            delay = self.base_delay * _fibonacci(n)
        else:
            delay = 0.0

        delay = min(delay, self.max_delay)

        if self.jitter:
            delay *= random.uniform(0.5, 1.5)

        return delay


DEFAULT_RETRY = RetryPolicy()
