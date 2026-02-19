"""
Dual-arm bracket assembly
==========================
Left arm holds bracket. Right arm inserts 4 bolts.
Both arms must reach position simultaneously for cycle time.

Gaps exposed:
  - No parallel execution (arms move sequentially → wasted time)
  - No nesting (bolt sub-workflow can't be reused as a single step)
  - No loops (4 bolts means copy-pasting 4 times)
  - No progress/lifecycle hooks (can't report "bolt 3 of 4")
"""

import asyncio
import relentless as rel
from relentless.adapters import LocalAdapter


# ── tasks ────────────────────────────────────────────────────────────

@rel.task(retry=3, timeout=5.0)
async def move_arm(ctx, arm: str, position: str):
    await ctx.execute(f"{arm}.move", position)

@move_arm.undo
async def _(ctx, arm: str, position: str):
    await ctx.execute(f"{arm}.move", "home")


@rel.task(timeout=3.0)
async def close_gripper(ctx, arm: str):
    ok = await ctx.execute(f"{arm}.gripper.close")
    if not ok:
        raise rel.TaskFailed(f"{arm} gripper failed")

@close_gripper.undo
async def _(ctx, arm: str):
    await ctx.execute(f"{arm}.gripper.open")


@rel.task(retry=2, timeout=10.0)
async def insert_bolt(ctx, hole_id: int):
    await ctx.execute("right.insert_bolt", hole_id)

@insert_bolt.undo
async def _(ctx, hole_id: int):
    await ctx.execute("right.retract_bolt", hole_id)


@rel.task(timeout=5.0)
async def tighten_bolt(ctx, hole_id: int, torque: float):
    await ctx.execute("right.tighten", hole_id, torque)

# GAP: tighten_bolt has no undo. In reality you'd loosen,
# but the undo needs the SAME hole_id that was tightened.
# That works here because undo gets the bound args — but what if
# the torque applied was different from requested? Undo doesn't
# get the *result*, only the original arguments.


# ── what we WANT to write ────────────────────────────────────────────
#
#   parallel(
#       move_arm("left", "hold_pos"),
#       move_arm("right", "approach"),
#   )
#   >> close_gripper("left")
#   >> for hole in [1, 2, 3, 4]:          # GAP: no loop primitive
#       insert_bolt(hole) >> tighten_bolt(hole, 5.0)
#
# ── what we CAN write ────────────────────────────────────────────────

assembly = rel.Sequence([
    # GAP: These two should run in parallel. Moving sequentially
    # wastes ~5 seconds of cycle time per assembly.
    move_arm("left", "hold_position"),
    move_arm("right", "approach_position"),

    close_gripper("left"),

    # GAP: No loop. Must unroll manually. Adding a 5th bolt means
    # editing this list. Error-prone, not data-driven.
    insert_bolt(1),
    tighten_bolt(1, 5.0),
    insert_bolt(2),
    tighten_bolt(2, 5.0),
    insert_bolt(3),
    tighten_bolt(3, 5.0),
    insert_bolt(4),
    tighten_bolt(4, 5.0),
])

# GAP: The (insert + tighten) pair is a logical sub-workflow.
# We can't extract it as a reusable Sequence and nest it inside
# the outer Sequence, because Sequence is not a valid step type.
# bolt_workflow = rel.Sequence([insert_bolt(1), tighten_bolt(1, 5.0)])
# assembly = rel.Sequence([..., bolt_workflow, ...])  # TypeError


async def main():
    adapter = LocalAdapter()
    for action in [
        "left.move", "right.move", "left.gripper.close",
        "left.gripper.open", "right.insert_bolt",
        "right.retract_bolt", "right.tighten",
    ]:
        adapter.on(action, lambda *a: True)

    result = await assembly.run(adapter)
    print(f"Assembly: {'OK' if result.success else 'FAILED'}")
    print(f"Total steps executed: {result.tasks_completed}")
    # GAP: No lifecycle hooks. Can't print "bolt 3/4 done" during
    # execution. No way to feed progress to a dashboard.

if __name__ == "__main__":
    asyncio.run(main())
