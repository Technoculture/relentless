"""Tests for task creation, execution, retry, timeout, and compensation."""

import pytest

import relentless as rel
from relentless.adapters import LocalAdapter


@pytest.mark.asyncio
async def test_task_basic():
    log = []

    @rel.task
    async def greet(ctx, name):
        log.append(f"hello {name}")

    adapter = LocalAdapter()
    ctx = rel.Context(adapter=adapter)
    await greet("world").run(ctx)
    assert log == ["hello world"]


@pytest.mark.asyncio
async def test_task_with_options():
    log = []

    @rel.task(retry=2, timeout=5.0)
    async def step(ctx, x):
        log.append(x)

    adapter = LocalAdapter()
    ctx = rel.Context(adapter=adapter)
    await step(42).run(ctx)
    assert log == [42]


@pytest.mark.asyncio
async def test_task_undo():
    log = []

    @rel.task
    async def open_valve(ctx, valve_id):
        log.append(f"open {valve_id}")

    @open_valve.undo
    async def _(ctx, valve_id):
        log.append(f"close {valve_id}")

    adapter = LocalAdapter()
    ctx = rel.Context(adapter=adapter)
    bound = open_valve("v1")
    await bound.run(ctx)
    await bound.compensate(ctx)
    assert log == ["open v1", "close v1"]


@pytest.mark.asyncio
async def test_task_no_undo_is_noop():
    @rel.task
    async def no_undo(ctx):
        pass

    adapter = LocalAdapter()
    ctx = rel.Context(adapter=adapter)
    bound = no_undo()
    await bound.run(ctx)
    # compensate should not raise
    await bound.compensate(ctx)


@pytest.mark.asyncio
async def test_task_retry_succeeds():
    attempts = []

    @rel.task(retry=3)
    async def flaky(ctx):
        attempts.append(1)
        if len(attempts) < 3:
            raise RuntimeError("not yet")

    adapter = LocalAdapter()
    ctx = rel.Context(adapter=adapter)
    await flaky().run(ctx)
    assert len(attempts) == 3


@pytest.mark.asyncio
async def test_task_retry_exhausted():
    @rel.task(retry=2)
    async def always_fail(ctx):
        raise RuntimeError("nope")

    adapter = LocalAdapter()
    ctx = rel.Context(adapter=adapter)
    with pytest.raises(rel.TaskFailed, match="always_fail"):
        await always_fail().run(ctx)


@pytest.mark.asyncio
async def test_task_timeout():
    import asyncio

    @rel.task(timeout=0.05)
    async def slow_task(ctx):
        await asyncio.sleep(10)

    adapter = LocalAdapter()
    ctx = rel.Context(adapter=adapter)
    with pytest.raises(rel.TaskFailed, match="slow_task"):
        await slow_task().run(ctx)


@pytest.mark.asyncio
async def test_task_uses_adapter():
    @rel.task
    async def move(ctx, dest):
        return await ctx.execute("arm.move", dest)

    adapter = LocalAdapter()
    adapter.on("arm.move", lambda dest: f"moved to {dest}")
    ctx = rel.Context(adapter=adapter)
    result = await move("bin").run(ctx)
    assert result == "moved to bin"
    assert adapter.calls == [("arm.move", ("bin",), {})]


def test_bound_task_repr():
    @rel.task
    async def move(ctx, dest):
        pass

    assert "move" in repr(move("bin"))
    assert "bin" in repr(move("bin"))
