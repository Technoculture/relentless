"""
Bin emptying
============
Pick every object from a bin and place on conveyor.
Object count isn't known until a vision scan at runtime.
Some objects may fail to pick (bad pose, too heavy) — skip them.

Gaps exposed:
  - No loops / iteration (sequence is fixed at definition time)
  - No conditional branching (can't skip bad objects)
  - No dynamic sequence construction from runtime data
  - No partial success (one bad object fails the whole sequence)
"""

import asyncio
import relentless as rel
from relentless.adapters import LocalAdapter


@rel.task(timeout=3.0)
async def scan_bin(ctx):
    """Vision system scans bin, returns list of object poses."""
    objects = await ctx.execute("vision.scan_bin")
    ctx.state["objects"] = objects or []
    # GAP: We store results in ctx.state with string keys.
    # No type safety. Typo "objcts" silently returns None later.

@rel.task(retry=2, timeout=5.0)
async def pick_object(ctx, obj_id: str, pose: tuple):
    await ctx.execute("arm.move", pose)
    ok = await ctx.execute("gripper.close")
    if not ok:
        raise rel.TaskFailed(f"Failed to pick {obj_id}")

@pick_object.undo
async def _(ctx, obj_id: str, pose: tuple):
    await ctx.execute("gripper.open")
    await ctx.execute("arm.move", "home")

@rel.task(timeout=5.0)
async def place_on_conveyor(ctx):
    await ctx.execute("arm.move", "conveyor_drop")
    await ctx.execute("gripper.open")

@place_on_conveyor.undo
async def _(ctx):
    # Can't un-place an object that's already on the conveyor.
    # Compensation for physical release is often a no-op or an alert.
    await ctx.execute("alert", "Object placed — cannot recall")


# ── what we WANT to write ────────────────────────────────────────────
#
#   scan_bin()
#   >> for obj in ctx.state["objects"]:     # GAP: no loop
#       try:
#           pick_object(obj.id, obj.pose)
#           >> place_on_conveyor()
#       except rel.TaskFailed:
#           continue                        # GAP: no skip-on-failure
#
# ── what we CAN write ────────────────────────────────────────────────

# WORKAROUND: Build the sequence dynamically with a plain Python loop.
# This works BUT: must build it AFTER scanning (two-phase execution),
# and if pick fails, the ENTIRE remaining sequence aborts.
# There is no "skip this object and continue with the next one."

async def run_bin_emptying(adapter):
    # Phase 1: scan
    ctx = rel.Context(adapter=adapter)
    await scan_bin().run(ctx)
    objects = ctx.state["objects"]
    print(f"Found {len(objects)} objects")

    # Phase 2: build sequence dynamically
    steps = []
    for obj in objects:
        steps.append(pick_object(obj["id"], tuple(obj["pose"])))
        steps.append(place_on_conveyor())

    if not steps:
        return

    pick_sequence = rel.Sequence(steps)

    # GAP: If object 3 of 5 fails to pick, everything stops.
    # Objects 4 and 5 never get attempted. We want "best-effort"
    # semantics — skip failures, report partial results.
    try:
        result = await pick_sequence.run(adapter)
        print(f"Picked all {len(objects)} objects")
    except rel.SequenceFailed as e:
        print(f"Failed at {e.failed_task_name}")
        # GAP: No way to know how many objects were successfully placed.
        # SequenceResult only exists on success. On failure we only get
        # the error, not "3 of 5 succeeded."


async def main():
    adapter = LocalAdapter()
    adapter.on("vision.scan_bin", lambda: [
        {"id": "part_A", "pose": (0.1, 0.2, 0.0)},
        {"id": "part_B", "pose": (0.3, 0.1, 0.0)},
        {"id": "part_C", "pose": (0.2, 0.4, 0.0)},  # this one will fail
        {"id": "part_D", "pose": (0.4, 0.3, 0.0)},
    ])
    adapter.on("arm.move", lambda pos: True)

    call_count = 0
    def gripper_close():
        nonlocal call_count
        call_count += 1
        if call_count == 5:  # 3rd pick attempt (after 2 retries)
            return False     # part_C fails
        return True

    adapter.on("gripper.close", gripper_close)
    adapter.on("gripper.open", lambda: True)
    adapter.on("alert", lambda msg: print(f"  ALERT: {msg}"))

    await run_bin_emptying(adapter)
    print(f"Total adapter calls: {len(adapter.calls)}")

if __name__ == "__main__":
    asyncio.run(main())
