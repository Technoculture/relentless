"""
Visual inspection and sorting
==============================
Camera inspects parts on a conveyor. Based on defect type:
  - No defect → pass to packaging
  - Cosmetic defect → route to rework station
  - Structural defect → reject bin
  - Unknown → stop line, call operator

Gaps exposed:
  - No conditional branching (can't route based on classification)
  - No error discrimination (all failures trigger same compensation)
  - No lifecycle hooks (can't report pass/fail stats to dashboard)
"""

import asyncio
import relentless as rel
from relentless.adapters import LocalAdapter


class DefectType:
    NONE = "none"
    COSMETIC = "cosmetic"
    STRUCTURAL = "structural"
    UNKNOWN = "unknown"


@rel.task(timeout=5.0)
async def capture_image(ctx, station_id: str):
    image = await ctx.execute("camera.capture", station_id)
    ctx.state["image"] = image


@rel.task(timeout=10.0)
async def classify_defect(ctx):
    image = ctx.state["image"]
    result = await ctx.execute("ml.classify", image)
    ctx.state["defect_type"] = result["type"]
    ctx.state["confidence"] = result["confidence"]


@rel.task(timeout=5.0)
async def route_to_packaging(ctx):
    await ctx.execute("conveyor.route", "packaging")

@rel.task(timeout=5.0)
async def route_to_rework(ctx):
    await ctx.execute("conveyor.route", "rework")

@rel.task(timeout=5.0)
async def route_to_reject(ctx):
    await ctx.execute("conveyor.route", "reject_bin")

@rel.task(timeout=30.0)
async def stop_and_call_operator(ctx):
    await ctx.execute("conveyor.stop")
    await ctx.execute("alert.operator", "Unknown defect — manual inspection")
    # In reality this would block waiting for operator input
    # GAP: No human-in-the-loop primitive.


# ── what we WANT to write ────────────────────────────────────────────
#
#   capture_image("station_1")
#   >> classify_defect()
#   >> branch(lambda ctx: ctx.state["defect_type"], {
#       DefectType.NONE:       route_to_packaging(),
#       DefectType.COSMETIC:   route_to_rework(),
#       DefectType.STRUCTURAL: route_to_reject(),
#       DefectType.UNKNOWN:    stop_and_call_operator(),
#   })
#
# ── what we CAN write ────────────────────────────────────────────────

# WORKAROUND: Stuff all logic into one big task. This works but
# defeats the purpose of relentless — no per-route compensation,
# no retry per route, no observability into which branch was taken.

@rel.task(timeout=30.0)
async def inspect_and_route(ctx, station_id: str):
    image = await ctx.execute("camera.capture", station_id)
    result = await ctx.execute("ml.classify", image)
    defect = result["type"]

    if defect == DefectType.NONE:
        await ctx.execute("conveyor.route", "packaging")
    elif defect == DefectType.COSMETIC:
        await ctx.execute("conveyor.route", "rework")
    elif defect == DefectType.STRUCTURAL:
        await ctx.execute("conveyor.route", "reject_bin")
    else:
        await ctx.execute("conveyor.stop")
        await ctx.execute("alert.operator", "Unknown defect")

    # GAP: This monolithic task can't have different compensation
    # per branch. If routing to rework fails, compensation should
    # be different than if routing to packaging fails.

inspection_flow = rel.Sequence([inspect_and_route("station_1")])


# ── error discrimination gap ─────────────────────────────────────────
#
# GAP: When a task raises an exception, all we get is a generic
# SequenceFailed. But different errors need different responses:
#
#   - CameraError       → retry with flash adjustment
#   - ClassifierError   → fall back to manual inspection
#   - ConveyorJamError  → stop line, alert maintenance
#   - SafetyViolation   → e-stop everything immediately
#
# Currently all errors trigger the same reverse compensation.
# There's no way to say "if this specific error type, do X instead."


# ── lifecycle hooks gap ──────────────────────────────────────────────
#
# GAP: In production you'd want:
#
#   flow.run(adapter, hooks=Hooks(
#       on_task_end=lambda name, ctx, result:
#           metrics.increment(f"inspection.{ctx.state.get('defect_type', 'unknown')}"),
#       on_task_error=lambda name, ctx, error:
#           logger.error(f"Inspection failed: {error}"),
#   ))
#
# No hook system exists. Can't report per-part statistics, track
# throughput, or alert on error rate spikes without wrapping
# everything manually.


async def main():
    adapter = LocalAdapter()
    adapter.on("camera.capture", lambda sid: {"raw": "image_data"})
    adapter.on("ml.classify", lambda img: {
        "type": DefectType.COSMETIC,
        "confidence": 0.92,
    })
    adapter.on("conveyor.route", lambda dest: print(f"  Routed to {dest}"))
    adapter.on("conveyor.stop", lambda: True)
    adapter.on("alert.operator", lambda msg: print(f"  OPERATOR: {msg}"))

    result = await inspection_flow.run(adapter)
    print(f"Inspection: {'OK' if result.success else 'FAILED'}")

if __name__ == "__main__":
    asyncio.run(main())
