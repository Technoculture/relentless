"""Tests for Guard and Branch combinators."""

import pytest
import relentless as rel
from relentless.adapters import LocalAdapter


# ── Guard ────────────────────────────────────────────────────────────

@pytest.mark.asyncio
async def test_guard_condition_true():
    log = []

    @rel.task
    async def action(ctx):
        log.append("ran")

    g = rel.Guard(lambda ctx: True, action())
    ctx = rel.Context(adapter=LocalAdapter())
    await g.run(ctx)
    assert log == ["ran"]


@pytest.mark.asyncio
async def test_guard_condition_false_raises():
    @rel.task
    async def action(ctx):
        pass

    g = rel.Guard(lambda ctx: False, action())
    ctx = rel.Context(adapter=LocalAdapter())

    with pytest.raises(rel.GuardFailed):
        await g.run(ctx)


@pytest.mark.asyncio
async def test_guard_condition_false_with_otherwise():
    log = []

    @rel.task
    async def primary(ctx):
        log.append("primary")

    @rel.task
    async def fallback(ctx):
        log.append("fallback")

    g = rel.Guard(lambda ctx: False, primary(), otherwise=fallback())
    ctx = rel.Context(adapter=LocalAdapter())
    await g.run(ctx)
    assert log == ["fallback"]


@pytest.mark.asyncio
async def test_guard_compensates_correct_branch():
    log = []

    @rel.task
    async def primary(ctx):
        log.append("run_primary")

    @primary.undo
    async def _(ctx):
        log.append("undo_primary")

    @rel.task
    async def fallback(ctx):
        log.append("run_fallback")

    @fallback.undo
    async def _(ctx):
        log.append("undo_fallback")

    # Condition false → runs fallback
    g = rel.Guard(lambda ctx: False, primary(), otherwise=fallback())
    ctx = rel.Context(adapter=LocalAdapter())
    await g.run(ctx)
    await g.compensate(ctx)
    assert "undo_fallback" in log
    assert "undo_primary" not in log


@pytest.mark.asyncio
async def test_guard_async_condition():
    log = []

    @rel.task
    async def action(ctx):
        log.append("ran")

    async def check(ctx):
        return ctx.state.get("ready", False)

    g = rel.Guard(check, action())
    ctx = rel.Context(adapter=LocalAdapter())
    ctx.state["ready"] = True
    await g.run(ctx)
    assert log == ["ran"]


@pytest.mark.asyncio
async def test_guard_inside_sequence():
    log = []

    @rel.task
    async def setup(ctx):
        ctx.state["force"] = 2.0
        log.append("setup")

    @setup.undo
    async def _(ctx):
        log.append("undo_setup")

    @rel.task
    async def insert(ctx):
        log.append("insert")

    @rel.task
    async def fails(ctx):
        raise RuntimeError("boom")

    flow = rel.Sequence([
        setup(),
        rel.Guard(lambda ctx: ctx.state["force"] < 5.0, insert()),
        fails(),
    ])

    with pytest.raises(rel.SequenceFailed):
        await flow.run(LocalAdapter())

    assert "insert" in log
    assert "undo_setup" in log


# ── Branch ───────────────────────────────────────────────────────────

@pytest.mark.asyncio
async def test_branch_routes_correctly():
    log = []

    @rel.task
    async def route_a(ctx):
        log.append("a")

    @rel.task
    async def route_b(ctx):
        log.append("b")

    b = rel.Branch(
        lambda ctx: ctx.state["choice"],
        {"a": route_a(), "b": route_b()},
    )
    ctx = rel.Context(adapter=LocalAdapter())
    ctx.state["choice"] = "b"
    await b.run(ctx)
    assert log == ["b"]


@pytest.mark.asyncio
async def test_branch_default():
    log = []

    @rel.task
    async def known(ctx):
        log.append("known")

    @rel.task
    async def fallback(ctx):
        log.append("fallback")

    b = rel.Branch(
        lambda ctx: "unknown_key",
        {"known": known()},
        default=fallback(),
    )
    ctx = rel.Context(adapter=LocalAdapter())
    await b.run(ctx)
    assert log == ["fallback"]


@pytest.mark.asyncio
async def test_branch_no_match_raises():
    @rel.task
    async def only_option(ctx):
        pass

    b = rel.Branch(
        lambda ctx: "missing",
        {"option": only_option()},
    )
    ctx = rel.Context(adapter=LocalAdapter())

    with pytest.raises(KeyError, match="missing"):
        await b.run(ctx)


@pytest.mark.asyncio
async def test_branch_compensates_chosen():
    log = []

    @rel.task
    async def route_a(ctx):
        log.append("run_a")

    @route_a.undo
    async def _(ctx):
        log.append("undo_a")

    @rel.task
    async def route_b(ctx):
        log.append("run_b")

    @route_b.undo
    async def _(ctx):
        log.append("undo_b")

    b = rel.Branch(
        lambda ctx: "a",
        {"a": route_a(), "b": route_b()},
    )
    ctx = rel.Context(adapter=LocalAdapter())
    await b.run(ctx)
    await b.compensate(ctx)
    assert "undo_a" in log
    assert "undo_b" not in log


@pytest.mark.asyncio
async def test_branch_inside_sequence():
    log = []

    @rel.task
    async def classify(ctx):
        ctx.state["defect"] = "cosmetic"

    @rel.task
    async def route_rework(ctx):
        log.append("rework")

    @rel.task
    async def route_pass(ctx):
        log.append("pass")

    flow = rel.Sequence([
        classify(),
        rel.Branch(
            lambda ctx: ctx.state["defect"],
            {
                "none": route_pass(),
                "cosmetic": route_rework(),
            },
            name="route_part",
        ),
    ])

    result = await flow.run(LocalAdapter())
    assert result.success
    assert log == ["rework"]
