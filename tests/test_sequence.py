"""Tests for sequence execution, compensation, and composition."""

import pytest

import relentless as rel
from relentless.adapters import LocalAdapter


@pytest.mark.asyncio
async def test_sequence_success():
    log = []

    @rel.task
    async def step_a(ctx):
        log.append("a")

    @rel.task
    async def step_b(ctx):
        log.append("b")

    result = await rel.Sequence([step_a(), step_b()]).run(LocalAdapter())
    assert result.success
    assert result.tasks_completed == 2
    assert log == ["a", "b"]


@pytest.mark.asyncio
async def test_sequence_compensation_on_failure():
    log = []

    @rel.task
    async def step_a(ctx):
        log.append("run_a")

    @step_a.undo
    async def _(ctx):
        log.append("undo_a")

    @rel.task
    async def step_b(ctx):
        log.append("run_b")

    @step_b.undo
    async def _(ctx):
        log.append("undo_b")

    @rel.task
    async def step_c(ctx):
        raise RuntimeError("fail")

    with pytest.raises(rel.SequenceFailed) as exc_info:
        await rel.Sequence([step_a(), step_b(), step_c()]).run(LocalAdapter())

    # Compensation runs in reverse order for completed tasks
    assert log == ["run_a", "run_b", "undo_b", "undo_a"]
    assert exc_info.value.failed_task_name == "step_c"
    assert exc_info.value.compensated  # all compensations succeeded


@pytest.mark.asyncio
async def test_sequence_first_task_fails():
    log = []

    @rel.task
    async def fails(ctx):
        raise RuntimeError("boom")

    @fails.undo
    async def _(ctx):
        log.append("undo_fails")

    with pytest.raises(rel.SequenceFailed):
        await rel.Sequence([fails()]).run(LocalAdapter())

    # No tasks completed, so no compensation should run
    assert log == []


@pytest.mark.asyncio
async def test_sequence_compensation_failure_recorded():
    @rel.task
    async def step_a(ctx):
        pass

    @step_a.undo
    async def _(ctx):
        raise RuntimeError("undo failed")

    @rel.task
    async def step_b(ctx):
        raise RuntimeError("b failed")

    with pytest.raises(rel.SequenceFailed) as exc_info:
        await rel.Sequence([step_a(), step_b()]).run(LocalAdapter())

    assert not exc_info.value.compensated
    assert len(exc_info.value.compensation_errors) == 1


@pytest.mark.asyncio
async def test_sequence_pipe_operator():
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

    flow = a() >> b() >> c()
    result = await flow.run(LocalAdapter())
    assert result.success
    assert log == ["a", "b", "c"]


@pytest.mark.asyncio
async def test_sequence_shared_state():
    @rel.task
    async def produce(ctx):
        ctx.state["value"] = 42

    @rel.task
    async def consume(ctx):
        assert ctx.state["value"] == 42

    result = await rel.Sequence([produce(), consume()]).run(LocalAdapter())
    assert result.success


@pytest.mark.asyncio
async def test_sequence_default_adapter():
    """Sequence can run without an explicit adapter (uses LocalAdapter)."""
    log = []

    @rel.task
    async def ping(ctx):
        log.append("ping")

    result = await rel.Sequence([ping()]).run()
    assert result.success
    assert log == ["ping"]


@pytest.mark.asyncio
async def test_sequence_with_adapter_calls():
    @rel.task
    async def move(ctx, dest):
        await ctx.execute("arm.move", dest)

    @rel.task
    async def grip(ctx):
        await ctx.execute("gripper.close")

    adapter = LocalAdapter()
    adapter.on("arm.move", lambda dest: True)
    adapter.on("gripper.close", lambda: True)

    result = await rel.Sequence([move("bin"), grip()]).run(adapter)
    assert result.success
    assert len(adapter.calls) == 2
    assert adapter.calls[0] == ("arm.move", ("bin",), {})
    assert adapter.calls[1] == ("gripper.close", (), {})
