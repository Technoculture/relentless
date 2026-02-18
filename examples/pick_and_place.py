"""Example: Pick and place with automatic compensation.

Run with: python -m asyncio examples/pick_and_place.py
"""

import asyncio

import relentless as rel
from relentless.adapters import LocalAdapter

SAFE_HOME = "safe_home"


@rel.task(retry=3, backoff="exponential", timeout=5.0)
async def move_to(ctx: rel.Context, destination: str):
    """Move robot arm to a named destination."""
    print(f"  Moving to {destination}...")
    await ctx.execute("arm.move", destination)


@move_to.undo
async def _(ctx: rel.Context, destination: str):
    print(f"  [UNDO] Moving arm to {SAFE_HOME}")
    await ctx.execute("arm.move", SAFE_HOME)


@rel.task(retry=2, timeout=2.0)
async def grasp(ctx: rel.Context):
    """Close gripper to grasp object."""
    print("  Grasping...")
    result = await ctx.execute("gripper.close")
    if result is False:
        raise rel.TaskFailed("Grip failed - object not detected")


@grasp.undo
async def _(ctx: rel.Context):
    print("  [UNDO] Opening gripper")
    await ctx.execute("gripper.open")


@rel.task
async def release(ctx: rel.Context):
    """Open gripper to release object."""
    print("  Releasing...")
    await ctx.execute("gripper.open")


# Compose into a workflow
pick_and_place = rel.Sequence([
    move_to("bin_a"),
    grasp(),
    move_to("conveyor"),
    release(),
])


async def main():
    # --- Successful run ---
    print("=== Successful run ===")
    adapter = LocalAdapter()
    adapter.on("arm.move", lambda dest: True)
    adapter.on("gripper.close", lambda: True)
    adapter.on("gripper.open", lambda: True)

    result = await pick_and_place.run(adapter)
    print(f"Result: {'OK' if result.success else 'FAILED'}")
    print(f"Actions: {[c[0] for c in adapter.calls]}")
    print()

    # --- Failed run with automatic compensation ---
    print("=== Failed run (grasp fails) ===")
    adapter = LocalAdapter()
    adapter.on("arm.move", lambda dest: True)
    adapter.on("gripper.close", lambda: False)  # Will cause grasp to fail
    adapter.on("gripper.open", lambda: True)

    try:
        await pick_and_place.run(adapter)
    except rel.SequenceFailed as e:
        print(f"Failed at: {e.failed_task_name}")
        print(f"Compensated: {e.compensated}")
        print(f"Actions: {[c[0] for c in adapter.calls]}")


if __name__ == "__main__":
    asyncio.run(main())
