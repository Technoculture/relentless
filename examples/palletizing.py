"""
Multi-robot palletizing
========================
3 robots pick from 3 conveyor lanes and place on a shared pallet.
They work in parallel but must coordinate when placing (shared zone).
The whole cycle must complete within 60 seconds.

Gaps exposed:
  - No parallel execution (robots can't work simultaneously)
  - No sequence-level timeout (can't enforce 60s cycle time)
  - No cancellation (can't abort a running workflow from outside)
  - No resource locking (shared pallet zone has no coordination)
"""

import asyncio
import relentless as rel
from relentless.adapters import LocalAdapter


# ── per-robot tasks ──────────────────────────────────────────────────

@rel.task(retry=2, timeout=5.0)
async def pick_from_conveyor(ctx, robot_id: int, lane: int):
    await ctx.execute(f"robot{robot_id}.move", f"lane{lane}_pick")
    ok = await ctx.execute(f"robot{robot_id}.gripper.close")
    if not ok:
        raise rel.TaskFailed(f"Robot {robot_id}: pick failed on lane {lane}")

@pick_from_conveyor.undo
async def _(ctx, robot_id: int, lane: int):
    await ctx.execute(f"robot{robot_id}.gripper.open")
    await ctx.execute(f"robot{robot_id}.move", "home")


@rel.task(timeout=8.0)
async def place_on_pallet(ctx, robot_id: int, slot: str):
    # GAP: In reality, only one robot can be in the pallet zone at a time.
    # We need a mutex/lock on "pallet_zone". Relentless has no
    # resource locking primitive. The user must roll their own:
    #
    #   async with pallet_lock:
    #       await ctx.execute(...)
    #
    # But that's inside the task body, invisible to the framework.
    # If the task fails while holding the lock, compensation doesn't
    # know to release it.
    await ctx.execute(f"robot{robot_id}.move", f"pallet_{slot}")
    await ctx.execute(f"robot{robot_id}.gripper.open")

@place_on_pallet.undo
async def _(ctx, robot_id: int, slot: str):
    # Can't un-place. Object is already on the pallet. Best effort:
    await ctx.execute("alert", f"Robot {robot_id}: object left at {slot}")


# ── per-robot workflow ───────────────────────────────────────────────

def make_robot_workflow(robot_id, lane, pallet_slot):
    return rel.Sequence([
        pick_from_conveyor(robot_id, lane),
        place_on_pallet(robot_id, pallet_slot),
    ])


robot1_flow = make_robot_workflow(1, 1, "A1")
robot2_flow = make_robot_workflow(2, 2, "A2")
robot3_flow = make_robot_workflow(3, 3, "A3")


# ── what we WANT to write ────────────────────────────────────────────
#
#   with timeout(60):                           # GAP: no sequence timeout
#       parallel(                               # GAP: no parallel
#           robot1_flow,
#           robot2_flow,
#           robot3_flow,
#       )
#
# ── what we CAN write ────────────────────────────────────────────────

# WORKAROUND 1: Run robots sequentially (3x slower)
sequential_palletize = rel.Sequence(
    robot1_flow.tasks + robot2_flow.tasks + robot3_flow.tasks
)

# WORKAROUND 2: Use raw asyncio.gather (loses all relentless benefits)
async def parallel_palletize_raw(adapter):
    """Manual parallel execution — no compensation, no retry tracking."""
    ctx1 = rel.Context(adapter=adapter)
    ctx2 = rel.Context(adapter=adapter)
    ctx3 = rel.Context(adapter=adapter)
    # GAP: Three separate contexts = three separate state dicts.
    # No shared state coordination. No unified compensation if
    # robot 2 fails after robots 1 and 3 succeed.
    results = await asyncio.gather(
        robot1_flow.run(adapter),
        robot2_flow.run(adapter),
        robot3_flow.run(adapter),
        return_exceptions=True,
    )
    for i, r in enumerate(results, 1):
        if isinstance(r, Exception):
            print(f"  Robot {i} failed: {r}")
            # GAP: Must manually compensate the other robots.
            # Relentless doesn't help here at all.
        else:
            print(f"  Robot {i}: OK")


# ── cancellation gap ─────────────────────────────────────────────────

# GAP: No way to abort a running workflow from the outside.
# If a safety system detects a problem mid-cycle, we can't cancel.
#
# In practice you'd want:
#
#   cancel_token = rel.CancelToken()
#   task = asyncio.create_task(flow.run(adapter, cancel=cancel_token))
#   ...
#   cancel_token.cancel()  # triggers compensation and stops
#
# Currently the only option is asyncio.Task.cancel(), which raises
# CancelledError with no compensation at all.


async def main():
    adapter = LocalAdapter()
    for rid in [1, 2, 3]:
        adapter.on(f"robot{rid}.move", lambda pos: True)
        adapter.on(f"robot{rid}.gripper.close", lambda: True)
        adapter.on(f"robot{rid}.gripper.open", lambda: True)
    adapter.on("alert", lambda msg: print(f"  ALERT: {msg}"))

    print("=== Sequential (slow) ===")
    result = await sequential_palletize.run(adapter)
    print(f"  Steps: {result.tasks_completed}, "
          f"Calls: {len(adapter.calls)}")

    print("\n=== Parallel (raw asyncio, no compensation) ===")
    adapter.reset()
    await parallel_palletize_raw(adapter)
    print(f"  Calls: {len(adapter.calls)}")

if __name__ == "__main__":
    asyncio.run(main())
