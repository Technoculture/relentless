"""Tests for Parallel execution."""

import pytest
import relentless as rel
from relentless.adapters import LocalAdapter


@pytest.mark.asyncio
async def test_parallel_all_succeed():
    log = []

    @rel.task
    async def a(ctx):
        log.append("a")

    @rel.task
    async def b(ctx):
        log.append("b")

    p = rel.Parallel([a(), b()])
    adapter = LocalAdapter()
    ctx = rel.Context(adapter=adapter)
    results = await p.run(ctx)
    assert set(log) == {"a", "b"}
    assert len(results) == 2


@pytest.mark.asyncio
async def test_parallel_one_fails_compensates_completed():
    import asyncio

    log = []

    @rel.task
    async def slow_ok(ctx):
        log.append("slow_start")
        await asyncio.sleep(0.05)
        log.append("slow_done")

    @slow_ok.undo
    async def _(ctx):
        log.append("slow_undo")

    @rel.task
    async def fast_fail(ctx):
        log.append("fast_fail")
        raise RuntimeError("boom")

    p = rel.Parallel([slow_ok(), fast_fail()])
    ctx = rel.Context(adapter=LocalAdapter())

    with pytest.raises(rel.ParallelFailed) as exc_info:
        await p.run(ctx)

    assert exc_info.value.failed_step_name == "fast_fail"
    # slow_ok may or may not have completed by the time fast_fail raises.
    # If it completed, it should be compensated.
    if "slow_done" in log:
        assert "slow_undo" in log


@pytest.mark.asyncio
async def test_parallel_inside_sequence():
    log = []

    @rel.task
    async def move_left(ctx):
        log.append("left")

    @move_left.undo
    async def _(ctx):
        log.append("undo_left")

    @rel.task
    async def move_right(ctx):
        log.append("right")

    @move_right.undo
    async def _(ctx):
        log.append("undo_right")

    @rel.task
    async def fails(ctx):
        raise RuntimeError("after parallel")

    flow = rel.Sequence([
        rel.Parallel([move_left(), move_right()], name="move_both"),
        fails(),
    ])

    with pytest.raises(rel.SequenceFailed):
        await flow.run(LocalAdapter())

    # Both parallel tasks completed, so both should be compensated
    assert "left" in log
    assert "right" in log
    assert "undo_left" in log
    assert "undo_right" in log


@pytest.mark.asyncio
async def test_parallel_preserves_result_order():
    """Results are ordered by input position, not completion order."""
    import asyncio

    @rel.task
    async def delayed(ctx, val, delay):
        await asyncio.sleep(delay)
        return val

    p = rel.Parallel([
        delayed("first", 0.05),
        delayed("second", 0.01),  # finishes first
    ])
    ctx = rel.Context(adapter=LocalAdapter())
    results = await p.run(ctx)
    assert results == ["first", "second"]
