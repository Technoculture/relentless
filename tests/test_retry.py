"""Tests for retry policy delay calculations."""

from relentless import RetryPolicy


def test_no_backoff():
    p = RetryPolicy(max_attempts=3, backoff="none")
    assert p.delay_for_attempt(1) == 0.0
    assert p.delay_for_attempt(2) == 0.0
    assert p.delay_for_attempt(3) == 0.0


def test_first_attempt_always_zero():
    p = RetryPolicy(max_attempts=5, backoff="exponential", base_delay=10.0)
    assert p.delay_for_attempt(1) == 0.0


def test_linear_backoff():
    p = RetryPolicy(max_attempts=5, backoff="linear", base_delay=1.0)
    assert p.delay_for_attempt(1) == 0.0
    assert p.delay_for_attempt(2) == 1.0
    assert p.delay_for_attempt(3) == 2.0
    assert p.delay_for_attempt(4) == 3.0


def test_exponential_backoff():
    p = RetryPolicy(max_attempts=5, backoff="exponential", base_delay=1.0)
    assert p.delay_for_attempt(1) == 0.0
    assert p.delay_for_attempt(2) == 1.0
    assert p.delay_for_attempt(3) == 2.0
    assert p.delay_for_attempt(4) == 4.0
    assert p.delay_for_attempt(5) == 8.0


def test_fibonacci_backoff():
    p = RetryPolicy(max_attempts=6, backoff="fibonacci", base_delay=1.0)
    assert p.delay_for_attempt(1) == 0.0
    assert p.delay_for_attempt(2) == 1.0  # fib(1) = 1
    assert p.delay_for_attempt(3) == 1.0  # fib(2) = 1
    assert p.delay_for_attempt(4) == 2.0  # fib(3) = 2
    assert p.delay_for_attempt(5) == 3.0  # fib(4) = 3
    assert p.delay_for_attempt(6) == 5.0  # fib(5) = 5


def test_max_delay_cap():
    p = RetryPolicy(
        max_attempts=10, backoff="exponential", base_delay=1.0, max_delay=5.0
    )
    assert p.delay_for_attempt(10) == 5.0


def test_jitter_stays_in_range():
    p = RetryPolicy(
        max_attempts=3, backoff="linear", base_delay=10.0, jitter=True
    )
    for _ in range(100):
        delay = p.delay_for_attempt(2)  # base * 1 = 10.0, with jitter
        assert 5.0 <= delay <= 15.0


def test_frozen_policy():
    """RetryPolicy is immutable."""
    p = RetryPolicy(max_attempts=3)
    try:
        p.max_attempts = 5  # type: ignore[misc]
        assert False, "Should have raised"
    except AttributeError:
        pass
