# Design: Compensation Algebra for Task Sequences

This document formalizes the compensation model behind Relentless and
maps each mathematical concept to its Rust implementation.

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
a : Context → Result<()>
```

A **compensation** `γ(a)` is an action that mitigates or reverses `a`:

```
γ : Action → Action
```

For reversible actions: `execute(γ(a)) after execute(a) ≈ no-op`
For irreversible actions: `γ(a)` is the best available mitigation.

**In code:**

```rust
let grasp = FnTask::new("grasp", |ctx| Box::pin(async move {
    ctx.execute("gripper.close", &[]).await?;
    Ok(())
})).with_compensate(|ctx| Box::pin(async move {
    ctx.execute("gripper.open", &[]).await?;
    Ok(())
}));
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

```rust
let flow = Sequence::new("assembly")
    .step(a1).step(a2).step(a3).step(a4);
// If a3 fails: compensate a2, then a1
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

```rust
let seq = Sequence::new("retrying")
    .step(flaky_step)
    .retry(RetryPolicy::exponential(5, Duration::from_millis(100)).with_jitter());
```

### Error Discrimination

Not all failures require the same response. The **error strategy**
function maps errors to behaviors:

```
σ : Error → {Compensate, Skip, Escalate}
```

- **Compensate**: undo completed steps in reverse (default)
- **Skip**: ignore this failure, continue to next step
- **Escalate**: stop immediately, do NOT compensate

**In code:**

```rust
fn error_strategy(&self, error: &Error) -> ErrorStrategy {
    match error {
        Error::TaskFailed { message, .. } if message.contains("non-critical") => {
            ErrorStrategy::Skip
        }
        _ => ErrorStrategy::Compensate,
    }
}
```

### Parallel Composition

Actions `a₁, a₂, ..., aₙ` execute concurrently:

```
P(a₁, ..., aₙ) : all aᵢ run simultaneously
```

If any `aᵢ` fails, all completed siblings `aⱼ` (j ≠ i, succeeded) are
compensated.

**In code:**

```rust
let par = Parallel::new("both_arms")
    .step(left_arm).step(right_arm);
```

### Conditional Execution

A **guard** `G(p, a, f)` checks predicate `p`, runs action `a` if true,
fallback `f` if false:

```
G(p, a, f) = if p(ctx) then a else f
```

A **branch** `B(s, [a₁, ..., aₙ])` selects one action based on
selector `s`:

```
B(s, branches) = branches[s(ctx)]
```

### Iteration

A **loop** `L(p, a, n)` repeats action `a` while predicate `p` holds,
bounded by maximum iterations `n`:

```
L(p, a, n) = while p(ctx) ∧ count < n: execute(a)
```

**In code:**

```rust
let fill = Loop::new("fill_pallet", pick_one, |ctx| {
    Box::pin(async move { ctx.get_bool("more_items").await })
}).max_iterations(100);
```

### Properties

**Compensation idempotency** (ideal): compensating twice has the same
effect as compensating once.

```
γ(a) ∘ γ(a) ≈ γ(a)
```

**Compensation ordering**: for independent actions, compensation order
doesn't matter. For dependent actions, reverse order preserves safety.

**Composition**: all combinators (`Sequence`, `Parallel`, `Guard`,
`Branch`, `Loop`) implement `Step`, so they nest freely:

```rust
let workflow = Sequence::new("complex")
    .step(Parallel::new("setup")
        .step(left_arm_sequence)
        .step(right_arm_sequence))
    .step(Guard::new("check", insert, condition)
        .with_fallback(abort))
    .step(Loop::new("fill", place_one, more_items));
```

## Adapter Abstraction

The **adapter** is a function that maps action names to physical effects:

```
adapter : (ActionName, Args) → Result<Value>
```

This decouples task logic from transport. The same sequence runs against:

- `LocalAdapter` → in-memory (testing)
- `ZenohAdapter` → Zenoh pub/sub (production)
- `ROS2Adapter` → ROS2 services (production)
- Any custom adapter implementing the trait

**In code:**

```rust
#[async_trait]
pub trait Adapter: Send + Sync {
    async fn execute(&self, action: &str, args: &[Value]) -> Result<Value>;
    async fn read(&self, key: &str) -> Result<Value>;
    async fn subscribe(&self, topic: &str) -> Result<mpsc::Receiver<Value>> { ... }
}
```

## Persistence and Recovery

A **journal** records step lifecycle events for crash recovery:

```
journal : (workflow_id, step_name, event) → ()
```

Events: `Started`, `Completed`, `Failed(reason)`, `Compensated`.

On restart with the same `workflow_id`, completed steps are skipped.
This implements the **at-most-once** execution guarantee for
completed steps.

## Resource Coordination

A **resource lock** serializes access to shared physical resources
across parallel steps:

```
locked(a, μ) = acquire(μ) → execute(a) → release(μ)
```

Where `μ` is a named mutex. Multiple steps sharing the same `μ`
execute sequentially even within a `Parallel`.

**In code:**

```rust
let zone = ResourceLock::new("pallet_zone");
let a = Locked::new(place_left, zone.clone());
let b = Locked::new(place_right, zone);
```

## Cancellation

A **cancellation token** provides cooperative shutdown:

```
cancel : () → set(cancelled)
check  : () → if cancelled then Error::Cancelled
```

Steps check cancellation at boundaries (between steps, between loop
iterations). On cancellation, the current sequence compensates
completed steps.
