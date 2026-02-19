"""Tests for lifecycle hooks."""

import pytest
import relentless as rel
from relentless.adapters import LocalAdapter


@pytest.mark.asyncio
async def test_hooks_on_step_start_and_end():
    events = []

    @rel.task
    async def a(ctx):
        pass

    @rel.task
    async def b(ctx):
        pass

    hooks = rel.Hooks(
        on_step_start=lambda name, ctx: events.append(f"start:{name}"),
        on_step_end=lambda name, ctx: events.append(f"end:{name}"),
    )

    await rel.Sequence([a(), b()]).run(LocalAdapter(), hooks=hooks)
    assert events == ["start:a", "end:a", "start:b", "end:b"]


@pytest.mark.asyncio
async def test_hooks_on_error_and_compensate():
    events = []

    @rel.task
    async def ok(ctx):
        pass

    @ok.undo
    async def _(ctx):
        pass

    @rel.task
    async def fail(ctx):
        raise RuntimeError("boom")

    hooks = rel.Hooks(
        on_step_error=lambda name, ctx, err: events.append(f"error:{name}"),
        on_compensate=lambda name, ctx: events.append(f"comp:{name}"),
    )

    with pytest.raises(rel.SequenceFailed):
        await rel.Sequence([ok(), fail()]).run(LocalAdapter(), hooks=hooks)

    assert events == ["error:fail", "comp:ok"]


@pytest.mark.asyncio
async def test_hooks_async():
    events = []

    @rel.task
    async def a(ctx):
        pass

    async def on_start(name, ctx):
        events.append(f"async_start:{name}")

    hooks = rel.Hooks(on_step_start=on_start)
    await rel.Sequence([a()]).run(LocalAdapter(), hooks=hooks)
    assert events == ["async_start:a"]


@pytest.mark.asyncio
async def test_hooks_can_track_progress():
    """Simulate a dashboard progress tracker using hooks."""
    progress = {"current": 0, "total": 3}

    @rel.task
    async def step(ctx, n):
        pass

    def track(name, ctx):
        progress["current"] += 1

    hooks = rel.Hooks(on_step_end=track)
    flow = rel.Sequence([step(1), step(2), step(3)])
    await flow.run(LocalAdapter(), hooks=hooks)
    assert progress["current"] == 3
