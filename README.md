# Relentless

**Robots fail. Relentless tasks don't.**

A lightweight Python library for composing async tasks with automatic
compensation on failure. Built for robotics, useful anywhere actions
need structured undo.

```bash
pip install relentless
```

Requires Python 3.10+. No other dependencies.

## Quick Start

```python
import relentless as rel

@rel.task
async def move_to(ctx, destination: str):
    await ctx.execute("arm.move", destination)

@move_to.undo
async def _(ctx, destination: str):
    await ctx.execute("arm.move", "home")

@rel.task
async def grasp(ctx):
    await ctx.execute("gripper.close")

@grasp.undo
async def _(ctx):
    await ctx.execute("gripper.open")

@rel.task
async def release(ctx):
    await ctx.execute("gripper.open")

# Compose into a sequence
flow = rel.Sequence([
    move_to("bin_a"),
    grasp(),
    move_to("conveyor"),
    release(),
])

# Run it
result = await flow.run(adapter)
```

If `move_to("conveyor")` fails, relentless automatically compensates in
reverse order:

1. Undo `grasp` &rarr; opens gripper
2. Undo `move_to("bin_a")` &rarr; moves arm home

The failed task (`move_to("conveyor")`) is not compensated because it
never completed.

## Core Concepts

### Tasks

A task is an async function decorated with `@rel.task`. Call it with
arguments to get a bound task ready for composition:

```python
@rel.task(retry=3, backoff="exponential", timeout=5.0)
async def move_to(ctx: rel.Context, destination: str):
    await ctx.execute("arm.move", destination)
```

### Compensation

Register an undo function with `@task.undo`. It receives the same
arguments as the original task:

```python
@move_to.undo
async def _(ctx: rel.Context, destination: str):
    await ctx.execute("arm.move", "safe_home")
```

Compensation is **declared, not inferred**. You always see exactly what
will undo what.

For safety-critical tasks, compensation can retry:

```python
@rel.task(retry=2, undo_retry=3)
async def close_gripper(ctx):
    await ctx.execute("gripper.close")

@close_gripper.undo
async def _(ctx):
    # Will retry up to 3 times if undo fails (e.g. gripper stuck)
    await ctx.execute("gripper.open")
```

### Sequences

A `Sequence` runs steps in order. On failure, it compensates all
completed steps in reverse:

```python
flow = rel.Sequence([
    move_to("station_1"),
    pick_up(),
    move_to("station_2"),
    place_down(),
])
```

Or use the `>>` operator:

```python
flow = move_to("station_1") >> pick_up() >> move_to("station_2") >> place_down()
```

Sequences nest. A sequence used inside another sequence acts as a single
step &mdash; if the outer sequence fails later, the inner sequence
compensates all its sub-steps:

```python
pick_phase = rel.Sequence([scan_bin(), pick_object()], name="pick")
place_phase = rel.Sequence([move_to("drop"), release()], name="place")
full_flow = rel.Sequence([pick_phase, place_phase])
```

### Parallel

Run steps concurrently. If any step fails, completed siblings are
compensated:

```python
flow = rel.Sequence([
    rel.Parallel([
        move_arm("left", "hold_position"),
        move_arm("right", "approach"),
    ], name="position_arms"),
    close_gripper("left"),
    insert_bolt(),
])
```

### Branching

Route execution based on runtime data:

```python
flow = rel.Sequence([
    classify_defect(),
    rel.Branch(
        lambda ctx: ctx.state["defect_type"],
        {
            "none":       route_to_packaging(),
            "cosmetic":   route_to_rework(),
            "structural": route_to_reject(),
        },
        default=stop_and_call_operator(),
        name="route_part",
    ),
])
```

### Guards

Run a step only if a precondition holds:

```python
flow = rel.Sequence([
    read_force_sensor(),
    rel.Guard(
        lambda ctx: ctx.state["force"] < 5.0,
        insert_peg(),
        otherwise=abort_insertion(),
    ),
])
```

### Retry Policies

Tasks can retry with configurable backoff:

```python
@rel.task(retry=5, backoff="exponential", base_delay=0.5, jitter=True)
async def unreliable_sensor_read(ctx):
    data = await ctx.execute("sensor.read")
    if data is None:
        raise rel.TaskFailed("No reading")
    return data
```

Backoff strategies: `"none"`, `"linear"`, `"exponential"`, `"fibonacci"`.

### Adapters

Adapters bridge tasks to the outside world. Relentless is
transport-agnostic &mdash; use whatever fits your system.

**LocalAdapter** for testing (no external dependencies):

```python
from relentless.adapters import LocalAdapter

adapter = LocalAdapter()
adapter.on("arm.move", lambda dest: print(f"Moving to {dest}"))
adapter.on("gripper.close", lambda: True)

result = await flow.run(adapter)

# Inspect what happened
assert ("arm.move", ("bin_a",), {}) in adapter.calls
```

**Custom adapters** implement two methods:

```python
class MyROS2Adapter:
    async def execute(self, action: str, *args, **kwargs):
        # Map action names to ROS2 service calls
        ...

    async def read(self, key: str):
        # Read from ROS2 topics
        ...
```

### Lifecycle Hooks

Monitor execution in production:

```python
hooks = rel.Hooks(
    on_step_start=lambda name, ctx: print(f"Starting {name}"),
    on_step_end=lambda name, ctx: metrics.increment(f"step.{name}.ok"),
    on_step_error=lambda name, ctx, err: logger.error(f"{name}: {err}"),
    on_compensate=lambda name, ctx: logger.warning(f"Compensating {name}"),
)

result = await flow.run(adapter, hooks=hooks)
```

### Shared State

Tasks share a `ctx.state` dict within a sequence:

```python
@rel.task
async def detect_object(ctx):
    pose = await ctx.execute("vision.detect")
    ctx.state["target_pose"] = pose

@rel.task
async def move_to_target(ctx):
    pose = ctx.state["target_pose"]
    await ctx.execute("arm.move", pose)
```

## Error Handling

When a task fails after exhausting retries, `SequenceFailed` is raised
with full context:

```python
try:
    result = await flow.run(adapter)
except rel.SequenceFailed as e:
    print(f"Failed at: {e.failed_task_name}")
    print(f"Error: {e.original_error}")
    print(f"Compensation errors: {e.compensation_errors}")
    print(f"All compensations succeeded: {e.compensated}")
```

## Known Limitations

Relentless is a task sequencer, not a complete robotics framework.
These are real gaps, documented honestly:

| Gap | Workaround | Status |
|:----|:-----------|:-------|
| No loops / iteration | Build sequences dynamically with Python `for` | By design |
| No cancellation | `asyncio.Task.cancel()` (no compensation) | Planned |
| No sequence-level timeout | Wrap with `asyncio.wait_for()` | Planned |
| No persistence / crash recovery | External journal | Planned |
| No partial success | Run items individually, collect results | Planned |
| No resource locking | Use `asyncio.Lock` inside task bodies | External |
| No error discrimination | Catch specific exceptions in task bodies | Planned |
| No sensor streaming | Out of scope (use motion controllers) | By design |
| `ctx.state` is untyped | Use descriptive keys, or dataclasses | Accepted |

See `examples/` for realistic scenarios that exercise these boundaries.

## Design Principles

1. **Library, not framework.** You call relentless. It doesn't
   restructure your code.
2. **No dependencies.** Core library needs only Python 3.10+. Adapters
   are optional.
3. **Testable by default.** Every workflow runs with `LocalAdapter()`,
   no hardware or network needed.
4. **Transport-agnostic.** Zenoh, ROS2, MQTT, direct calls &mdash; plug
   in what you use.
5. **Explicit over magic.** Compensation is declared, not inferred.

## License

MIT. See [LICENSE](LICENSE).
