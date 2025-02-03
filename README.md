# Relentless: Unbreakable Workflows for Real-World Robotics

[![Python 3.10+](https://img.shields.io/badge/python-3.10+-blue.svg)](https://www.python.org/)
[![Zenoh 0.8+](https://img.shields.io/badge/zenoh-0.8+-orange.svg)](https://zenoh.io/)

**Robots fail. Relentless workflows don't.**

Relentless is a Python framework for building robust, fault-tolerant workflows that thrive in the chaos of real-world robotics. It provides a powerful and intuitive way to handle inevitable failures, so your robots keep working even when things go wrong.

## Why Your Robots Need Relentless

*   **They drop things.**
*   **Their sensors lie.**
*   **Networks are flaky.**
*   **The unexpected happens.**

Traditional workflow systems crumble under these pressures. Relentless was built for the challenge.

## Key Features

*   **Actionable Compensation:** Define how to undo actions or mitigate their effects when steps fail.
*   **Smart Retries:** Configurable backoff strategies (linear, exponential, Fibonacci) with jitter.
*   **Time-Aware:** Wall-clock timeouts, sensor-based triggers, and time-bound compensation.
*   **Stateful Execution:** Leverages Zenoh's built-in persistence for transparent, versioned state management.
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
Relentless is built on a formal mathematical model that ensures predictable behavior and robust error recovery. This model is reflected in the Python API through the following concepts:

### 1. Workflows
A `Workflow` is a sequence of `Steps` with defined success and failure paths. Each `Workflow` defines a `Compensation Strategy` to be used in case of failure during workflow execution.

```python
from relentless import workflow, Workflow, CompensateReverse, CompensateAction

@workflow
class PickAndPlace(Workflow):
    compensation_strategy = CompensateReverse()

    def build(self):
      return [
          MoveTo("bin"),
          Grasp(),
          MoveTo("conveyor"),
          Release()
      ]
```

### 2. Steps
A `Step` is an individual, potentially reversible, action within a `Workflow`. Each `Step` defines how it is executed (`.run()`) as well as how it is compensated (`.compensate()`). Each `Step` can define its own `RetryPolicy` and `TimeoutPolicy`.

```python
from relentless import Step, RetryPolicy, TimeoutPolicy, Compensation, WorkflowContext

class MoveTo(Step):
    def __init__(self, destination: str):
        super().__init__(
            name=f"move_to_{destination}",
            retry_policy=RetryPolicy(
                max_attempts=5,
                backoff="exponential"
            ),
            timeout_policy=TimeoutPolicy(
                timeout=timedelta(seconds=5)
            )
        )
        self.destination = destination

    async def run(self, context: WorkflowContext):
        await context.zenoh.put(
            f"/robot/{context.workflow_id}/arm/target",
            self.destination.encode()
        )

    async def compensate(self, context: WorkflowContext):
        await context.zenoh.put(
            f"/robot/{context.workflow_id}/arm/home",
            b''  # Empty payload triggers default "home" action
        )

class Grasp(Step):
    def __init__(self):
        super().__init__(
            name="grasp",
            retry_policy=RetryPolicy(
                max_attempts=3
            ),
            timeout_policy=TimeoutPolicy(
                timeout=timedelta(seconds=2)
            )
        )

    async def run(self, context: WorkflowContext):
        result = await context.zenoh.get(
            f"/robot/{context.workflow_id}/gripper/close"
        )
        if not result:
            raise GripperError("Failed to close gripper")
        
    async def compensate(self, context: WorkflowContext):
        await context.zenoh.put(
            f"/robot/{context.workflow_id}/gripper/open",
            b''
        )
```

### 3. Compensation
Each `Step` can define a compensation action using `Compensation` or a `CompensateAction`, which is executed if the `Step` fails or if a later `Step` in the `Workflow` fails. `Compensation` can be defined as either `CompensateReverse`, which executes the `.compensate()` method of each `Step` in reverse order, `CompensateAction`, which defines a custom compensation action, or a custom `Compensation` strategy.

```python
from relentless import Step, Compensation, CompensateAction, WorkflowContext

class LogCompensation(CompensateAction):
  def __init__(self, message: str):
    super().__init__(name='log_compensation')
    self.message = message
      
  async def compensate(self, context: WorkflowContext):
      await context.zenoh.put(
          f"/logs/{context.workflow_id}",
          self.message
      )

class LogStep(Step):
    def __init__(self, log_message: str):
        super().__init__(
            name="log_step",
            compensation=LogCompensation(message=log_message)
        )

    async def run(self, context: WorkflowContext):
        await context.zenoh.put(
            f"/logs/{context.workflow_id}",
            "Log step executed successfully"
        )
```

## Real-World Example: Bin Picking with Failure Recovery
```python
from relentless import (
    workflow, Workflow, Step, CompensateReverse,
    CompensateAction, WorkflowContext, WorkflowExecutor,
    RetryPolicy, TimeoutPolicy, Atomic
)
from zenoh import Zenoh
import numpy as np

async def emergency_stop(zenoh: Zenoh):
    await zenoh.put("/robot/emergency_stop", b'')

async def log_error(zenoh: Zenoh, message: str):
    await zenoh.put(f"/errors/{context.workflow_id}", message)

class MoveTo(Step):
    def __init__(self, destination: str):
        super().__init__(
            name=f"move_to_{destination}",
            retry_policy=RetryPolicy(
                max_attempts=5,
                backoff="exponential"
            ),
            timeout_policy=TimeoutPolicy(
                timeout=timedelta(seconds=5)
            )
        )
        self.destination = destination

    async def run(self, context: WorkflowContext):
        await context.zenoh.put(
            f"/arm/{context.workflow_id}/target_pose",
            self.destination.tobytes()
        )

    async def compensate(self, context: WorkflowContext):
        await context.zenoh.put(
            f"/arm/{context.workflow_id}/target_pose",
            SAFE_POSE.tobytes() # compensation is to move back to safe pose
        )

class Grasp(Step):
    def __init__(self, bin_id: str):
        super().__init__(
            name="grasp",
            retry_policy=RetryPolicy(
                max_attempts=3
            ),
            timeout_policy=TimeoutPolicy(
                timeout=timedelta(seconds=2)
            )
        )
        self.bin_id = bin_id

    async def run(self, context: WorkflowContext):
        await context.zenoh.put(f"/gripper/{self.bin_id}/cmd", "close")
        
    async def compensate(self, context: WorkflowContext):
        await context.zenoh.put(f"/gripper/{self.bin_id}/cmd", "emergency_release")

@workflow
class BinPicking(Workflow):
    compensation_strategy = CompensateReverse()

    def __init__(self, bin_id: str):
        super().__init__(name=f"bin_picking_{bin_id}")
        self.bin_id = bin_id

    def build(self):
        async def check_vision_confidence(context: WorkflowContext):
            confidence = await context.zenoh.get(f"/vision/{self.bin_id}/confidence", timeout=2.0)
            if confidence < 0.7:
                raise VisionError("Part not clearly visible")

        async def check_weight(context: WorkflowContext):
            weight = await context.zenoh.get("/load_cell/weight")
            if weight > 20.0:
                await context.zenoh.put(f"/gripper/{self.bin_id}/cmd", "release")
                raise HeavyObjectError(f"Object too heavy: {weight}kg")

        return [
            Atomic(
                name="grab_part",
                steps=[
                    Step(name="check_vision", run=check_vision_confidence, retry_policy=RetryPolicy(max_attempts=1), compensation=CompensateAction(emergency_stop)),
                    MoveTo(calculate_pose_from_vision(self.bin_id)),
                    Grasp(self.bin_id)
                ],
                on_failure=emergency_stop(zenoh) # emergency stop if atomic fails
            ),
            Step(name="verify_grip", run=check_weight, retry_policy=RetryPolicy(attempts=2), timeout_policy=TimeoutPolicy(seconds=3)),
            MoveTo(get_container_pose()),
            Release(self.bin_id)
        ]

async def main():
    zenoh = await Zenoh.connect()
    executor = WorkflowExecutor(zenoh)

    # Example of running the workflow
    await executor.run(BinPicking("cell1"))

```

## Advanced Features
### 1. Timeouts
```python
from relentless import TimeoutPolicy, WorkflowContext, Step
from datetime import timedelta

class Grasp(Step):
    # ...
    timeout_policy = TimeoutPolicy(
        timeout=timedelta(seconds=2), # or specify a function: timeout=lambda context: context.workflow_config.grasp_timeout
        on_timeout=emergency_stop  # Or define custom logic: on_timeout=lambda context: context.zenoh.put(...)
    )
    # ...
```

### 2. Atomic Blocks
Atomic blocks use the defined `on_failure` action if any `Step` within the block fails. This does not prevent the normal compensation logic from executing.

```python
from relentless import Atomic, Step, WorkflowContext

async def release_and_alert(context: WorkflowContext):
    await context.zenoh.put(f"/gripper/{context.workflow_id}/cmd", "release")
    await context.zenoh.put(f"/alerts/{context.workflow_id}", "Heavy object detected")

Atomic(
    name="load_sensing",
    steps=[
      Step(name="check_weight", run=check_weight, retry_policy=RetryPolicy(attempts=2), timeout_policy=TimeoutPolicy(seconds=3))
    ],
    on_failure=release_and_alert
)
```

### 3. Human Escalation
```python
from relentless import Step, WorkflowContext

class WaitForHuman(Step):
    def __init__(self):
        super().__init__(name="wait_for_human")

    async def run(self, context: WorkflowContext):
        while True:
            response = await context.zenoh.get(f"/human/{context.workflow_id}/response")
            if response and response.value.decode() == "OK":
                break
            await asyncio.sleep(5)

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
        await WaitForHuman()
```

## Configuration
Relentless uses Zenoh's built-in persistence mechanisms. You can configure persistence using standard Zenoh router configuration files.

**Example `config.json5`:**
```json5
{
    mode: 'router',
    plugins: {
        rest: {
            port: 8000
        },
        storage_manager: {
            storages: {
                workflow_state: {
                    volume: {
                        backend: 'rocksdb',
                        path: '/tmp/zenoh-storage-workflow'
                    }
                }
            }
        }
    },
    // other config options
}
```
Refer to the [Zenoh documentation](https://zenoh.io/docs/getting-started/quick-start/) for more details on configuring Zenoh.

## Monitoring
```python
async def monitor_workflows(zenoh: Zenoh):
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
