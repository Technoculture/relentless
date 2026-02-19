"""
Emergency stop and recovery
============================
Normal pick-and-place running. E-stop fires mid-cycle.
Must interrupt immediately, compensate safely.
Compensation itself might fail (gripper mechanically stuck).
Eventually need a human to intervene.

Gaps exposed:
  - No interrupt / cancellation mechanism
  - No compensation retry (undo that fails is just logged)
  - No compensation timeout (stuck undo blocks forever)
  - No human-in-the-loop escalation
  - No persistence (can't resume after power cycle)
"""

import asyncio
import relentless as rel
from relentless.adapters import LocalAdapter


@rel.task(retry=3, timeout=5.0)
async def move_arm(ctx, position: str):
    await ctx.execute("arm.move", position)

@move_arm.undo
async def _(ctx, position: str):
    # In an e-stop scenario this might timeout — the arm might be
    # locked. GAP: No timeout on compensation. If the arm servo is
    # disabled (e-stop cuts power), this hangs forever.
    await ctx.execute("arm.move", "safe_home")


@rel.task(retry=2, timeout=3.0)
async def close_gripper(ctx):
    ok = await ctx.execute("gripper.close")
    if not ok:
        raise rel.TaskFailed("Gripper failed to close")

@close_gripper.undo
async def _(ctx):
    # GAP: If the gripper is mechanically stuck, this fails.
    # Currently the failure is recorded in SequenceFailed.compensation_errors
    # but there's no retry on compensation. One failure = give up.
    # In safety-critical robotics, compensation MUST be retried.
    await ctx.execute("gripper.open")


@rel.task(timeout=10.0)
async def do_work(ctx, work_id: str):
    """Simulate a long-running operation that might get interrupted."""
    await ctx.execute("process.start", work_id)
    # GAP: If an e-stop fires HERE, there's no mechanism to interrupt
    # this task. asyncio.Task.cancel() raises CancelledError but
    # relentless doesn't catch it or trigger compensation.
    await asyncio.sleep(5)  # long operation
    await ctx.execute("process.finish", work_id)


cycle = rel.Sequence([
    move_arm("pick_position"),
    close_gripper(),
    move_arm("work_position"),
    do_work("weld_seam_1"),
    move_arm("place_position"),
])


# ── what we WANT ─────────────────────────────────────────────────────
#
#   token = rel.CancelToken()
#
#   # In another coroutine (e-stop handler):
#   async def estop_handler():
#       await wait_for_estop_signal()
#       token.cancel()  # triggers compensation in the running workflow
#
#   result = await cycle.run(adapter, cancel=token)
#
# ── what we CAN do ───────────────────────────────────────────────────

async def run_with_estop_simulation(adapter):
    """Simulate e-stop by cancelling the asyncio task."""
    run_task = asyncio.create_task(cycle.run(adapter))

    # Simulate e-stop after 1 second
    await asyncio.sleep(0.01)

    # This is the only interruption mechanism available:
    run_task.cancel()

    try:
        await run_task
    except asyncio.CancelledError:
        print("  Workflow cancelled by e-stop")
        # GAP: No compensation happened. The arm might be mid-motion,
        # the gripper closed on an object. CancelledError bypasses
        # relentless entirely. The system is in an unknown state.
        print("  WARNING: No compensation was triggered!")
        print("  WARNING: Robot may be in unsafe state!")


# ── compensation retry gap ────────────────────────────────────────────

async def run_with_stuck_gripper(adapter):
    """Show what happens when compensation fails."""
    # Make gripper.open fail (mechanically stuck)
    adapter.on("gripper.open", lambda: (_ for _ in ()).throw(
        RuntimeError("Gripper mechanically stuck — needs manual reset")
    ))

    # Make the work task fail, triggering compensation
    adapter.on("process.start", lambda wid: (_ for _ in ()).throw(
        RuntimeError("Weld power supply fault")
    ))

    try:
        await cycle.run(adapter)
    except rel.SequenceFailed as e:
        print(f"  Failed at: {e.failed_task_name}")
        print(f"  Compensation OK: {e.compensated}")
        if not e.compensated:
            for err in e.compensation_errors:
                print(f"  Compensation error: {err}")
            # GAP: Compensation failed. Now what?
            # - No retry on the stuck gripper
            # - No escalation to a human operator
            # - No persistent record that this happened
            # - Process restarts → no idea what state we're in
            print("  UNRESOLVED: Gripper stuck, no retry, no escalation")


# ── persistence gap ──────────────────────────────────────────────────
#
# GAP: If the process crashes (power outage, OOM kill, segfault):
#   - Which tasks completed? Unknown.
#   - What needs compensation? Unknown.
#   - Can we resume from where we left off? No.
#
# All state is in-memory. There's no write-ahead log, no checkpoint,
# no way to recover. For a library targeting robotics where power
# failures are real, this is a significant gap.


async def main():
    adapter = LocalAdapter()
    adapter.on("arm.move", lambda pos: True)
    adapter.on("gripper.close", lambda: True)
    adapter.on("gripper.open", lambda: True)
    adapter.on("process.start", lambda wid: True)
    adapter.on("process.finish", lambda wid: True)

    print("=== E-stop simulation ===")
    await run_with_estop_simulation(adapter)

    print("\n=== Stuck gripper (compensation failure) ===")
    adapter.reset()
    adapter.on("arm.move", lambda pos: True)
    adapter.on("gripper.close", lambda: True)
    await run_with_stuck_gripper(adapter)

if __name__ == "__main__":
    asyncio.run(main())
