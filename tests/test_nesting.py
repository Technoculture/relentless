"""Tests for nested sequences and compensation retry."""

import pytest
import relentless as rel
from relentless.adapters import LocalAdapter


@pytest.mark.asyncio
async def test_nested_sequence_runs():
    log = []

    @rel.task
    async def a(ctx):
        log.append("a")

    @rel.task
    async def b(ctx):
        log.append("b")

    @rel.task
    async def c(ctx):
        log.append("c")

    inner = rel.Sequence([a(), b()], name="inner")
    outer = rel.Sequence([inner, c()])
    result = await outer.run(LocalAdapter())

    assert result.success
    assert log == ["a", "b", "c"]


@pytest.mark.asyncio
async def test_nested_sequence_compensates_on_outer_failure():
    log = []

    @rel.task
    async def a(ctx):
        log.append("run_a")

    @a.undo
    async def _(ctx):
        log.append("undo_a")

    @rel.task
    async def b(ctx):
        log.append("run_b")

    @b.undo
    async def _(ctx):
        log.append("undo_b")

    @rel.task
    async def fails(ctx):
        raise RuntimeError("outer fails")

    inner = rel.Sequence([a(), b()], name="inner")
    outer = rel.Sequence([inner, fails()])

    with pytest.raises(rel.SequenceFailed):
        await outer.run(LocalAdapter())

    # Inner sequence should compensate b then a
    assert log == ["run_a", "run_b", "undo_b", "undo_a"]


@pytest.mark.asyncio
async def test_nested_sequence_compensates_on_inner_failure():
    log = []

    @rel.task
    async def a(ctx):
        log.append("run_a")

    @a.undo
    async def _(ctx):
        log.append("undo_a")

    @rel.task
    async def fails(ctx):
        raise RuntimeError("inner fails")

    @rel.task
    async def never(ctx):
        log.append("never")

    inner = rel.Sequence([a(), fails()], name="inner")
    outer = rel.Sequence([inner, never()])

    with pytest.raises(rel.SequenceFailed):
        await outer.run(LocalAdapter())

    assert "run_a" in log
    assert "undo_a" in log
    assert "never" not in log


@pytest.mark.asyncio
async def test_nested_shares_context_state():
    @rel.task
    async def produce(ctx):
        ctx.state["val"] = 42

    @rel.task
    async def consume(ctx):
        assert ctx.state["val"] == 42

    inner = rel.Sequence([produce()], name="inner")
    outer = rel.Sequence([inner, consume()])
    result = await outer.run(LocalAdapter())
    assert result.success


# ── compensation retry ────────────────────────────────────────────────

@pytest.mark.asyncio
async def test_undo_retry_succeeds_on_second_attempt():
    log = []
    undo_attempts = []

    @rel.task(undo_retry=3)
    async def risky(ctx):
        log.append("run")

    @risky.undo
    async def _(ctx):
        undo_attempts.append(1)
        if len(undo_attempts) < 2:
            raise RuntimeError("undo flaky")
        log.append("undo_ok")

    @rel.task
    async def trigger_fail(ctx):
        raise RuntimeError("fail")

    with pytest.raises(rel.SequenceFailed) as exc_info:
        await rel.Sequence([risky(), trigger_fail()]).run(LocalAdapter())

    assert exc_info.value.compensated  # undo succeeded on retry
    assert "undo_ok" in log
    assert len(undo_attempts) == 2


@pytest.mark.asyncio
async def test_undo_retry_exhausted():
    undo_attempts = []

    @rel.task(undo_retry=2)
    async def risky(ctx):
        pass

    @risky.undo
    async def _(ctx):
        undo_attempts.append(1)
        raise RuntimeError("permanently stuck")

    @rel.task
    async def trigger_fail(ctx):
        raise RuntimeError("fail")

    with pytest.raises(rel.SequenceFailed) as exc_info:
        await rel.Sequence([risky(), trigger_fail()]).run(LocalAdapter())

    assert not exc_info.value.compensated
    assert len(undo_attempts) == 2
