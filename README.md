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

### Sequences

A `Sequence` runs tasks in order. On failure, it compensates all
completed tasks in reverse:

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
transport-agnostic&mdash;use whatever fits your system.

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

## Real-World Example: Bin Picking

```python
import relentless as rel
from relentless.adapters import LocalAdapter

SAFE_HOME = "safe_home"

@rel.task(retry=3, backoff="exponential", timeout=5.0)
async def move_to(ctx: rel.Context, destination: str):
    await ctx.execute("arm.move", destination)

@move_to.undo
async def _(ctx: rel.Context, destination: str):
    await ctx.execute("arm.move", SAFE_HOME)

@rel.task(retry=1)
async def check_vision(ctx: rel.Context, bin_id: str):
    confidence = await ctx.execute("vision.detect", bin_id)
    if confidence is not None and confidence < 0.7:
        raise rel.TaskFailed("Part not clearly visible")
    ctx.state["target_pose"] = await ctx.execute("vision.pose", bin_id)

@rel.task(retry=2, timeout=2.0)
async def grasp(ctx: rel.Context):
    result = await ctx.execute("gripper.close")
    if result is False:
        raise rel.TaskFailed("Grip failed")

@grasp.undo
async def _(ctx: rel.Context):
    await ctx.execute("gripper.open")

@rel.task
async def verify_weight(ctx: rel.Context):
    weight = await ctx.execute("loadcell.read")
    if weight is not None and weight > 20.0:
        await ctx.execute("gripper.open")
        raise rel.TaskFailed(f"Object too heavy: {weight}kg")

@rel.task
async def release(ctx: rel.Context):
    await ctx.execute("gripper.open")

# Compose the full workflow
bin_pick = rel.Sequence([
    check_vision("cell_1"),
    move_to("bin_pose"),
    grasp(),
    verify_weight(),
    move_to("conveyor"),
    release(),
])

# Test it without any hardware
async def main():
    adapter = LocalAdapter()
    adapter.on("arm.move", lambda dest: True)
    adapter.on("vision.detect", lambda bin_id: 0.95)
    adapter.on("vision.pose", lambda bin_id: [0.5, 0.3, 0.1])
    adapter.on("gripper.close", lambda: True)
    adapter.on("gripper.open", lambda: True)
    adapter.on("loadcell.read", lambda: 5.0)

    result = await bin_pick.run(adapter)
    assert result.success
    print(f"Completed {result.tasks_completed} tasks")
```

## Design Principles

1. **Library, not framework.** You call relentless. It doesn't
   restructure your code.
2. **No dependencies.** Core library needs only Python 3.10+. Adapters
   are optional.
3. **Testable by default.** Every workflow runs with `LocalAdapter()`,
   no hardware or network needed.
4. **Transport-agnostic.** Zenoh, ROS2, MQTT, direct calls&mdash;plug
   in what you use.
5. **Explicit over magic.** Compensation is declared, not inferred.

## License

MIT. See [LICENSE](LICENSE).
