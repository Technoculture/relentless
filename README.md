# Relentless: Unbreakable Workflows for Real-World Robotics

[![Zenoh 0.8+](https://img.shields.io/badge/zenoh-0.8+-orange)](https://zenoh.io/)
[![Python 3.10+](https://img.shields.io/badge/python-3.10+-blue)](https://www.python.org/)

**Robots fail. Relentless workflows don't.**

Relentless is a Python framework for building robust, fault-tolerant workflows that thrive in the chaos of real-world robotics. It provides battle-tested tools for handling inevitable failures, so your robots keep working even when things go wrong.

## Why Your Robots Need Relentless
*   **They drop things.**
*   **Their sensors lie.**
*   **Networks are flaky.**
*   **The unexpected happens.**

Traditional workflow systems crumble under these pressures. Relentless is built for the challenge.

## Key Features That Keep You in Production

*   **Compensation Chains:** Define how to undo actions when (not if) steps fail.
*   **Smart Retries:** Configurable backoff strategies (linear, exponential, Fibonacci) with jitter.
*   **Time-Aware:** Wall-clock timeouts, sensor-based triggers, and time-bound compensation.
*   **Stateful Execution:** Transparent persistence on Zenoh-KV with versioned updates.
*   **Partial Rollbacks:** Undo only what's needed, not the entire workflow.
*   **Human-in-the-Loop:** Escalate to operators when automation hits its limits.
*   **Distributed Coordination:** Built on Zenoh for seamless multi-robot orchestration.
*   **Physical-World Ready:** Designed for irreversible actions, unreliable sensors, and real-world surprises.

## Installation

```bash
pip install relentless-flow
```

**Requirements:**
*   Python 3.10+

## Core Concepts
### 1. Workflows
A sequence of steps with defined success and failure paths:

```python
from relentless import workflow

@workflow
async def pick_and_place(item_id: str):
    await move_to_bin(item_id).retry(3)
    await grasp(item_id).compensate(release())
    await move_to_conveyor(item_id)
    await release()
```

### 2. Steps
Individual actions with optional retries, timeouts, and compensation:

```python
from relentless import Step, RetryPolicy, Compensation

class MoveTo(Step):
    def __init__(self, destination: str):
        super().__init__(
            name=f"move_to_{destination}",
            retry_policy=RetryPolicy(
                max_attempts=5,
                backoff="exponential"
            )
        )

    async def run(self, context):
        await context.zenoh.put(
            f"/robot/{context.robot_id}/arm/target",
            destination.encode()
        )

    async def compensate(self, context):
        await context.zenoh.put(
            f"/robot/{context.robot_id}/arm/home",
            b''  # Empty payload triggers default "home" action
        )

class Grasp(Step):
    def __init__(self):
        super().__init__(name="grasp")

    async def run(self, context):
        result = await context.zenoh.get(
            f"/robot/{context.robot_id}/gripper/close",
            timeout=2.0
        )
        if not result.success:
            raise GripperError("Failed to close gripper")
        
    async def compensate(self, context):
        await context.zenoh.put(
            f"/robot/{context.robot_id}/gripper/open",
            b''
        )
```

### 3. Compensation Strategies
Define *how* to recover from failures:

```python
from relentless import (
    ReverseOrderStrategy, 
    DependencyAwareStrategy,
    PhysicalMitigationStrategy,
    LayeredStrategy
)

# Default: undo steps in reverse order
pick_workflow = Workflow(
    steps=[MoveTo("bin"), Grasp(), MoveTo("conveyor")],
    compensation_strategy=ReverseOrderStrategy()
)

# For complex dependencies:
assembly_workflow = Workflow(
    # ... steps with intricate dependencies
    compensation_strategy=DependencyAwareStrategy(
        dependency_graph=my_dependency_graph
    )
)

# When things really break:
emergency_workflow = Workflow(
    steps=[MoveArm(), Weld(), Inspect()],
    compensation_strategy=LayeredStrategy([
        PhysicalMitigationStrategy(
            actions=[
                lambda ctx: ctx.zenoh.put("/safety/stop_all", b''),
                lambda ctx: ctx.zenoh.put("/alarm/siren", b'on')
            ]
        ),
        ReverseOrderStrategy()  # Try reversing after mitigation
    ])
)
```

## Real-World Example: Bin Picking with Failure Recovery
```python
from relentless import workflow, retry, compensate, atomic, WorkflowExecutor
from zenoh import Zenoh
import numpy as np

@workflow
async def bin_picking(zenoh: Zenoh, bin_id: str):
    """
    1. Find part in bin using computer vision.
    2. Attempt pick with force monitoring.
    3. Verify grip using weight sensor.
    4. Place part in container.
    5. Handle heavy object detection (20kg+).
    """
    ARM_CMD = f"/arm/{bin_id}/target_pose"
    GRIPPER_CMD = f"/gripper/{bin_id}/cmd"
    FORCE_FEEDBACK = f"/sensors/{bin_id}/force"
    VISION_CONF = f"/vision/{bin_id}/confidence"

    async with atomic("Grab part or abort"):
        confidence = await zenoh.get(VISION_CONF, timeout=2.0)
        if confidence < 0.7:
            raise VisionError("Part not clearly visible")

        target_pose = calculate_pose(confidence)
        await zenoh.put(ARM_CMD, target_pose.tobytes()).retry(
            strategy='fibonacci',
            max_attempts=3,
            on_failure=emergency_stop(zenoh)
        ).compensate(
            zenoh.put(ARM_CMD, SAFE_POSE.tobytes())
            >> log_error(f"Failed move to {target_pose}")
        )

        grip_task = (
            zenoh.put(GRIPPER_CMD, "close")
            .timeout(1.5, "Gripper jammed")
            .retry(2)
            .with_force_check(
                sensor=FORCE_FEEDBACK,
                min=15.0,
                max=45.0,
                window=timedelta(seconds=2)
            )
        )
        await grip_task.compensate(
            zenoh.put(GRIPPER_CMD, "emergency_release")
            >> zenoh.put("/alarms/gripper_fault", bin_id)
        )

    try:
        weight = await zenoh.get("/load_cell/weight")
        if weight > 20.0:
            await zenoh.put(GRIPPER_CMD, "release")
            raise HeavyObjectError(f"Object too heavy: {weight}kg")

        await verify_grip(zenoh, bin_id).retry(2, backoff=1.0).timeout(3.0)

    except (HeavyObjectError, VisionVerifyError):
        await zenoh.put("/alarms/heavy_object", bin_id)
        await log_to_mes("Heavy part detected - manual check")
        raise

    place_pose = get_container_pose()
    await zenoh.put(ARM_CMD, place_pose.tobytes()).retry(2)
    await zenoh.put(GRIPPER_CMD, "release")

async def main():
    zenoh = await Zenoh.connect()
    executor = WorkflowExecutor(
        zenoh,
        policy=ProductionPolicy(
            max_retries=5,
            human_escalation_timeout=300
        )
    )

    async with executor.monitor("/workflows/bin_picking/**") as live:
        async for update in live:
            if update.status == "COMPENSATING":
                play_alert_sound()
            post_to_mes(f"Bin {update.bin_id} | {update.current_step}")

    async with TaskGroup() as tg:
        for bin_id in ["cell1", "cell2", ..., "cell10"]:
            tg.create_task(executor.run(bin_picking, zenoh, bin_id))

```

## Advanced Features
### 1. Timeouts
```python
# Wall-clock timeout
await do_something().timeout(seconds=5, on_timeout=handle_timeout)

# Sensor-based timeout
await grip_object().with_timeout(
    condition=lambda ctx: ctx.zenoh.get("/force_sensor") > 50.0,
    on_timeout=release_gripper
)
```

### 2. Partial Compensation
```python
async with atomic("Move and drill"):
    await move_arm(target).compensate(move_arm(safe_pose))
    await drill_hole().compensate(
        fallback=lambda ctx: log_error("Failed to seal hole")
    )
    
# Only move_arm compensation is executed if drill_hole fails
```

### 3. Human Escalation
```python
@workflow
async def critical_process():
    try:
        await sensitive_operation().retry(1)
    except Exception as e:
        await notify_operator(e).timeout(
            seconds=300,
            on_timeout=shutdown_system
        )
        # Wait for human to resolve
        await wait_for_human_intervention()

async def wait_for_human_intervention():
    while True:
        response = await zenoh.get("/operator/response")
        if response == "OK":
            break
        await asyncio.sleep(10)
```

## Configuration
```python
from relentless import RelentlessConfig

config = RelentlessConfig(
    default_retry_policy=RetryPolicy(
        max_attempts=3,
        backoff="fibonacci"
    ),
    compensation_storage_ttl="1h",  # Keep compensation data for 1 hour
    safety_overrides={
        "max_arm_speed": 0.5,  # m/s
        "max_gripper_force": 50.0  # N
    }
)

Relentless.set_global_config(config)
```

## Monitoring
```python
async def monitor_workflows():
    async with zenoh.subscribe("relentless/state/**") as stream:
        async for update in stream:
            state = WorkflowState.parse_raw(update.value)
            print(f"Workflow {state.id} ({state.name}):")
            print(f"  Status: {state.status}")
            print(f"  Step: {state.current_step}/{len(state.steps)}")
            if state.errors:
                print(f"  Errors: {state.errors}")
```

## How Relentless Improves on Existing Systems
| Feature              | Relentless                                  | AWS Step Functions | Temporal.io | Zenoh Flow             |
| :------------------- | :------------------------------------------ | :---------------- | :---------- | :--------------------- |
| **Compensation**     | ✅ First-class, multi-strategy             | ❌ Limited          | ✅ Activities | ❌ Not a workflow system |
| **Real-World Focus** | ✅ Designed for physical robots            | ➖ Generic         | ➖ Generic    | ➖ Data-flow focused   |
| **Zenoh Integration** | ✅ Native state, pub/sub, geo-distribution | ❌ AWS-only        | ❌ Limited   | ✅                      |
| **Timeouts**         | ✅ Sensor-based + wall-clock               | ✅ Wall-clock only | ✅           | ✅                      |
| **Partial Rollback** | ✅ Fine-grained control                    | ❌ All-or-nothing   | ❌           | ❌                      |
| **Error Handling**   | ✅ Retries, timeouts, compensation, escalation | ✅ Retries, timeouts | ✅          | ✅                      |
| **State Management** | ✅ Versioned, on any Zenoh-KV              | ➖ DynamoDB        | ✅           | ✅                      |

## Contributing
We welcome contributions! To get started:

1. **Fork** the repository.
2. Create a feature branch: `git checkout -b feat/your-feature-name`
3. Commit your changes: `git commit -m 'Add amazing new feature'`
4. Push to the branch: `git push origin feat/your-feature-name`
5. Open a **Pull Request**.

## License
Relentless is licensed under the MIT License. See [LICENSE](LICENSE) for details.

## Acknowledgments
*   The [Zenoh](https://zenoh.io/) team for their amazing work on distributed robotics communication.
*   Inspired by the challenges of real-world robotic deployments.
