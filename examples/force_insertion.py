"""
Force-controlled peg insertion
===============================
Insert a peg into a hole using force feedback.
If initial approach doesn't find the hole, do a spiral search.
Must monitor force continuously to avoid damage.

Gaps exposed:
  - No guards / preconditions (should check force before inserting)
  - No result passing between tasks (only stringly-typed ctx.state)
  - Undo function is per-Task, not per-BoundTask (shared across calls)
  - No sensor-driven waits (poll until condition)
  - Adapter protocol too narrow for subscriptions / streaming data
"""

import asyncio
import relentless as rel
from relentless.adapters import LocalAdapter


@rel.task(timeout=3.0)
async def move_to_approach(ctx, target: tuple):
    """Move to position above the insertion point."""
    await ctx.execute("arm.move", target)
    ctx.state["approach_pose"] = target

@move_to_approach.undo
async def _(ctx, target: tuple):
    await ctx.execute("arm.move", "home")


@rel.task(timeout=2.0)
async def read_force(ctx):
    """Sample the force/torque sensor."""
    force = await ctx.execute("ft_sensor.read")
    ctx.state["current_force"] = force
    # GAP: This reads force ONCE. Real insertion needs continuous
    # force monitoring throughout the motion. The adapter protocol
    # has execute() and read() — no subscribe() for streaming data.
    # A real force-controlled insertion is a tight control loop,
    # not a sequence of discrete tasks. This is where relentless
    # hits its architectural boundary: it's a task sequencer,
    # not a motion controller.


@rel.task(retry=3, timeout=10.0)
async def insert_peg(ctx):
    """Push peg downward with force limit."""
    # GAP: We WANT a precondition: "only insert if current force < 5N".
    # There's no guard primitive. We have to check manually:
    force = ctx.state.get("current_force", 0)
    if force > 5.0:
        raise rel.TaskFailed(f"Force too high before insertion: {force}N")

    result = await ctx.execute("arm.insert", {"force_limit": 20.0})
    if result == "contact_lost":
        raise rel.TaskFailed("Lost contact during insertion")
    ctx.state["insertion_depth"] = result

@insert_peg.undo
async def _(ctx):
    await ctx.execute("arm.retract", 50.0)  # retract 50mm


@rel.task(timeout=15.0)
async def spiral_search(ctx):
    """If direct insertion failed, spiral outward to find the hole."""
    result = await ctx.execute("arm.spiral_search", {
        "radius_mm": 5.0,
        "force_limit": 10.0,
    })
    if not result:
        raise rel.TaskFailed("Spiral search failed to find hole")
    ctx.state["corrected_pose"] = result

@spiral_search.undo
async def _(ctx):
    await ctx.execute("arm.retract", 50.0)


@rel.task(timeout=5.0)
async def verify_insertion(ctx):
    """Check that insertion depth is within tolerance."""
    depth = ctx.state.get("insertion_depth", 0)
    if depth < 45.0:
        raise rel.TaskFailed(f"Insertion too shallow: {depth}mm")


# ── what we WANT to write ────────────────────────────────────────────
#
#   move_to_approach(target_pose)
#   >> guard(lambda ctx: ctx.state["current_force"] < 5.0,
#            insert_peg())                  # GAP: no guard
#   >> on_fail(insert_peg(),                # GAP: no conditional
#              fallback=spiral_search())     #   "try A, else B"
#   >> verify_insertion()
#
# ── what we CAN write ────────────────────────────────────────────────

# Best we can do: a flat sequence with the guard check inlined into
# the task body. No fallback path — spiral_search would have to be
# in a separate try/except outside relentless.

insertion = rel.Sequence([
    move_to_approach((0.5, 0.3, 0.1)),
    read_force(),
    insert_peg(),
    verify_insertion(),
])


async def run_with_fallback(adapter):
    """WORKAROUND: Manual try/except for fallback to spiral search."""
    try:
        result = await insertion.run(adapter)
        print("Direct insertion succeeded")
        return result
    except rel.SequenceFailed as e:
        if "contact_lost" in str(e.original_error):
            print("Direct insert failed — trying spiral search")
            # GAP: We lost the execution context (ctx.state) when
            # the sequence failed. Must rebuild from scratch.
            fallback = rel.Sequence([
                move_to_approach((0.5, 0.3, 0.1)),
                spiral_search(),
                insert_peg(),
                verify_insertion(),
            ])
            return await fallback.run(adapter)
        raise


# ── undo is per-Task, not per-call ────────────────────────────────────

# GAP: move_to_approach's undo always moves to "home", regardless of
# which approach pose was used. If the same task is used multiple
# times with different args, the undo always receives the correct
# bound args — so this actually works for move_to_approach.
#
# But consider: what if undo needs the RESULT of the task, not
# just the input args? For example, insert_peg returns insertion
# depth — the undo might need to retract by exactly that depth.
# Currently undo can't access the task's return value.


async def main():
    adapter = LocalAdapter()
    adapter.on("arm.move", lambda pos: True)
    adapter.on("ft_sensor.read", lambda: 2.5)
    adapter.on("arm.insert", lambda params: 48.0)  # 48mm depth
    adapter.on("arm.retract", lambda mm: True)
    adapter.on("arm.spiral_search", lambda params: (0.51, 0.31, 0.1))

    result = await insertion.run(adapter)
    print(f"Insertion: {'OK' if result.success else 'FAILED'}")
    print(f"Adapter calls: {[c[0] for c in adapter.calls]}")

if __name__ == "__main__":
    asyncio.run(main())
