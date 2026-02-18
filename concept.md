# Design: Compensation Algebra for Task Sequences

This document formalizes the compensation model behind Relentless and
maps each mathematical concept to its implementation.

## The Problem

Robotic actions are often **irreversible** or have **physical side
effects**. When a multi-step operation fails midway, you can't roll back
a database transaction -- you need to physically undo what was done
(open the gripper, move the arm home, release the part).

The question: **how do you systematically undo a partial sequence of
physical actions?**

## The Model

### Actions and Compensation

An **action** `a` is an async operation that changes the physical world:

```
a : Context → Result
```

A **compensation** `γ(a)` is an action that mitigates or reverses `a`:

```
γ : Action → Action
```

For reversible actions: `execute(γ(a)) after execute(a) ≈ no-op`
For irreversible actions: `γ(a)` is the best available mitigation.

**In code:**

```python
@rel.task
async def grasp(ctx):          # action a
    await ctx.execute("gripper.close")

@grasp.undo
async def _(ctx):              # compensation γ(a)
    await ctx.execute("gripper.open")
```

### Sequences and the Saga Pattern

A **sequence** is an ordered list of actions:

```
S = [a₁, a₂, ..., aₙ]
```

Execution produces a trajectory of states:

```
s₀ →(a₁)→ s₁ →(a₂)→ s₂ → ... →(aₙ)→ sₙ
```

If action `aₖ` fails after `a₁...aₖ₋₁` succeeded, the **compensation
sequence** is:

```
C(S, k) = [γ(aₖ₋₁), γ(aₖ₋₂), ..., γ(a₁)]
```

We compensate completed actions in reverse order. The failed action is
excluded because it never completed.

**In code:**

```python
flow = rel.Sequence([a1, a2, a3, a4])
# If a3 fails: compensate a2, then a1
```

### Retry as Bounded Repetition

A **retry policy** `R(a, n, d)` executes action `a` up to `n` times
with delay function `d(attempt)`:

```
R(a, n, d) = a | delay(d(1)) → a | delay(d(2)) → a | ... | FAIL
```

Where `|` means "if failed, then". Delay strategies:

- **None**: `d(i) = 0`
- **Linear**: `d(i) = base × i`
- **Exponential**: `d(i) = base × 2^(i-1)`
- **Fibonacci**: `d(i) = base × fib(i)`

With optional jitter: `d'(i) = d(i) × uniform(0.5, 1.5)`

**In code:**

```python
@rel.task(retry=5, backoff="exponential", base_delay=0.5, jitter=True)
async def flaky_sensor(ctx):
    ...
```

### Properties

**Compensation idempotency** (ideal): compensating twice has the same
effect as compensating once.

```
γ(a) ∘ γ(a) ≈ γ(a)
```

**Compensation ordering**: for independent actions, compensation order
doesn't matter. For dependent actions, reverse order preserves safety.

**Composition**: sequences compose. If `S₁ = [a, b]` and `S₂ = [c, d]`,
then `S₁ >> S₂ = [a, b, c, d]` with compensation `[γ(d), γ(c), γ(b),
γ(a)]` on full failure.

## Adapter Abstraction

The **adapter** is a function that maps action names to physical effects:

```
adapter : (ActionName, Args) → Result
```

This decouples task logic from transport. The same sequence runs against:

- `LocalAdapter` → in-memory (testing)
- `ZenohAdapter` → Zenoh pub/sub (production)
- `ROS2Adapter` → ROS2 services (production)
- Any custom adapter implementing the protocol

**In code:**

```python
class Adapter(Protocol):
    async def execute(self, action: str, *args, **kwargs) -> Any: ...
    async def read(self, key: str) -> Any: ...
```

## Future Extensions

- **Atomic blocks**: group actions with a shared `on_failure` handler.
- **Dependency-aware compensation**: compensate based on a dependency
  DAG, not just reverse order.
- **Persistent state**: store execution progress for crash recovery.
- **Concurrent tasks**: execute independent tasks in parallel within a
  sequence.
- **Sensor-based timeouts**: timeout based on sensor readings, not just
  wall-clock.
