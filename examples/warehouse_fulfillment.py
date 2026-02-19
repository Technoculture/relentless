"""
Warehouse order fulfillment
============================
AMR (mobile robot) picks items from shelves to fulfill an order.
Order has N items from different aisles. Some may be out of stock.
Must handle partial fulfillment and nested sub-workflows.

Gaps exposed:
  - No nested workflows (navigate+pick is a reusable sub-unit)
  - No partial success / best-effort semantics
  - No dynamic task generation from data
  - ctx.state is untyped, fragile, and global to the sequence
"""

import asyncio
import relentless as rel
from relentless.adapters import LocalAdapter


# ── tasks ────────────────────────────────────────────────────────────

@rel.task(retry=2, timeout=30.0)
async def navigate_to(ctx, aisle: str):
    await ctx.execute("amr.navigate", aisle)

@navigate_to.undo
async def _(ctx, aisle: str):
    await ctx.execute("amr.navigate", "staging_area")


@rel.task(timeout=5.0)
async def scan_shelf(ctx, sku: str):
    """Check if item is available on shelf."""
    result = await ctx.execute("vision.find_sku", sku)
    if result is None:
        raise rel.TaskFailed(f"SKU {sku} not found on shelf")
    ctx.state[f"pose_{sku}"] = result
    # GAP: Using f"pose_{sku}" as key is fragile. A typo anywhere
    # silently produces None. No schema, no validation, no IDE support.


@rel.task(retry=2, timeout=10.0)
async def pick_item(ctx, sku: str):
    pose = ctx.state.get(f"pose_{sku}")
    await ctx.execute("arm.move", pose)
    ok = await ctx.execute("gripper.pick")
    if not ok:
        raise rel.TaskFailed(f"Failed to pick {sku}")
    # Track picked items for partial fulfillment reporting
    picked = ctx.state.setdefault("picked_items", [])
    picked.append(sku)

@pick_item.undo
async def _(ctx, sku: str):
    await ctx.execute("gripper.release")
    await ctx.execute("arm.move", "stow")


@rel.task(timeout=5.0)
async def place_in_tote(ctx, sku: str):
    await ctx.execute("arm.move", "tote_drop")
    await ctx.execute("gripper.release")


# ── composing a sub-workflow for one item ────────────────────────────

def pick_one_item(aisle: str, sku: str):
    """Navigate to aisle, find item, pick it, stow in tote."""
    return rel.Sequence([
        navigate_to(aisle),
        scan_shelf(sku),
        pick_item(sku),
        place_in_tote(sku),
    ])

# GAP: pick_one_item returns a Sequence. We can't use it as a step
# inside another Sequence. This means we can't compose:
#
#   order_flow = rel.Sequence([
#       pick_one_item("A3", "BOLT-M8"),    # TypeError
#       pick_one_item("B1", "NUT-M8"),     # TypeError
#       pick_one_item("C2", "WASHER-M8"),  # TypeError
#       navigate_to("packing_station"),
#   ])
#
# WORKAROUND: Flatten all tasks into one mega-sequence.


# ── building the order ───────────────────────────────────────────────

ORDER = [
    {"aisle": "A3", "sku": "BOLT-M8"},
    {"aisle": "B1", "sku": "NUT-M8"},
    {"aisle": "C2", "sku": "WASHER-M8"},
    {"aisle": "A3", "sku": "SPRING-12"},
    {"aisle": "D4", "sku": "PIN-6"},
]


def build_order_workflow(order_items):
    """Build a flat sequence for the entire order."""
    steps = []
    for item in order_items:
        steps.extend([
            navigate_to(item["aisle"]),
            scan_shelf(item["sku"]),
            pick_item(item["sku"]),
            place_in_tote(item["sku"]),
        ])
    steps.append(navigate_to("packing_station"))
    return rel.Sequence(steps)


# ── partial fulfillment gap ──────────────────────────────────────────

async def fulfill_order_best_effort(adapter, order_items):
    """WORKAROUND: Run items one at a time, skip failures."""
    picked = []
    failed = []
    for item in order_items:
        flow = pick_one_item(item["aisle"], item["sku"])
        try:
            await flow.run(adapter)
            picked.append(item["sku"])
        except rel.SequenceFailed:
            failed.append(item["sku"])
            print(f"  Skipping {item['sku']} (failed or out of stock)")
            # GAP: This works but loses ALL relentless benefits:
            # - Each item runs in its own context (no shared state)
            # - No unified compensation across items
            # - If item 3 fails, items 1-2 are already placed in tote
            #   with no way to un-place them through relentless

    print(f"  Picked: {picked}")
    print(f"  Failed: {failed}")
    print(f"  Fulfillment rate: {len(picked)}/{len(order_items)}")

    # GAP: SequenceResult only reports success=True/False.
    # No "partial success" state. No way to return
    # "3 of 5 items picked" through the result type.


# ── state collision gap ──────────────────────────────────────────────
#
# GAP: ctx.state is one flat dict shared by ALL tasks. If two items
# in the order use the same SKU, their state keys collide:
#
#   ctx.state["pose_BOLT-M8"] gets overwritten on second visit to A3
#
# There's no namespacing, no scoping, no isolation between
# sub-workflows running in the same context.


async def main():
    adapter = LocalAdapter()
    adapter.on("amr.navigate", lambda aisle: True)
    adapter.on("arm.move", lambda pos: True)
    adapter.on("gripper.pick", lambda: True)
    adapter.on("gripper.release", lambda: True)

    call_count = 0
    def find_sku(sku):
        nonlocal call_count
        call_count += 1
        if sku == "WASHER-M8":
            return None  # out of stock
        return (0.5, 0.3, 0.1)

    adapter.on("vision.find_sku", find_sku)

    print("=== Full order (all-or-nothing — fails on WASHER) ===")
    all_or_nothing = build_order_workflow(ORDER)
    try:
        result = await all_or_nothing.run(adapter)
        print(f"  All items picked: {result.tasks_completed} steps")
    except rel.SequenceFailed as e:
        print(f"  Failed at: {e.failed_task_name}")
        print(f"  Compensation ran: {e.compensated}")
        # All previously picked items get un-picked. Wasteful.

    print("\n=== Best-effort (manual workaround) ===")
    adapter.reset()
    call_count = 0
    await fulfill_order_best_effort(adapter, ORDER)

if __name__ == "__main__":
    asyncio.run(main())
