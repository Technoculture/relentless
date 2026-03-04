# Relentless

**Robots fail. Relentless tasks don't.**

A zero-dependency Rust library for composing async tasks with automatic
compensation on failure. Built for robotics, useful anywhere actions
need structured undo.

```toml
[dependencies]
relentless = "0.1"
```

Requires Rust 2021 edition. Only runtime dependency: `tokio` + `async-trait`.

## Quick Start

```rust
use relentless::*;

#[tokio::main]
async fn main() {
    let adapter = LocalAdapter::new();
    let ctx = Context::new(adapter);

    let workflow = Sequence::new("pick_and_place")
        .step(FnTask::new("pick", |ctx| Box::pin(async move {
            ctx.execute("gripper.close", &[]).await?;
            Ok(())
        })).with_compensate(|ctx| Box::pin(async move {
            ctx.execute("gripper.open", &[]).await?;
            Ok(())
        })))
        .step(FnTask::new("place", |ctx| Box::pin(async move {
            ctx.execute("move_to", &[Value::str("target")]).await?;
            ctx.execute("gripper.open", &[]).await?;
            Ok(())
        })));

    workflow.run(&ctx).await.unwrap();
}
```

If `place` fails, relentless automatically compensates in reverse:
1. Undo `pick` &rarr; opens gripper

The failed step is not compensated because it never completed.

## Core Concepts

### Step Trait

Everything implements `Step`: tasks, sequences, parallels, guards, branches,
loops. They nest freely.

```rust
#[async_trait]
pub trait Step: Send + Sync {
    fn name(&self) -> &str;
    async fn run(&self, ctx: &Context) -> Result<()>;
    async fn compensate(&self, ctx: &Context) -> Result<()> { Ok(()) }
    fn error_strategy(&self, _error: &Error) -> ErrorStrategy {
        ErrorStrategy::Compensate
    }
}
```

### FnTask (closure-based steps)

Build steps inline without implementing the trait:

```rust
let pick = FnTask::new("pick", |ctx| Box::pin(async move {
    ctx.execute("gripper.close", &[]).await?;
    Ok(())
})).with_compensate(|ctx| Box::pin(async move {
    ctx.execute("gripper.open", &[]).await?;
    Ok(())
}));
```

### Sequences

Run steps in order. On failure, compensate completed steps in reverse:

```rust
let workflow = Sequence::new("assembly")
    .step(pick)
    .step(move_to)
    .step(place)
    .retry(RetryPolicy::exponential(3, Duration::from_millis(100)))
    .timeout(Duration::from_secs(30));
```

Sequences nest. A sequence inside another sequence acts as a single step.

### Parallel

Run steps concurrently. If any fails, completed siblings are compensated:

```rust
let both_arms = Parallel::new("dual_arm")
    .step(left_arm_sequence)
    .step(right_arm_sequence);
```

### Branch

Route execution based on runtime state:

```rust
let sort = Branch::new("classify", |ctx| Box::pin(async move {
    match ctx.get_str("grade").await.as_deref() {
        Some("A") => 0,
        Some("B") => 1,
        _ => 2,
    }
}))
.branch(route_to_premium)
.branch(route_to_standard)
.branch(route_to_reject);
```

### Guard

Run a step only if a condition holds, with optional fallback:

```rust
let guarded = Guard::new("check_force", insert_peg, |ctx| {
    Box::pin(async move { ctx.get_f64("force").await.unwrap_or(999.0) < 5.0 })
})
.with_fallback(abort_insertion);
```

### Loop

Repeat a step while a condition is true:

```rust
let fill = Loop::new("fill_pallet", pick_and_place_one, |ctx| {
    Box::pin(async move {
        ctx.get("items").await.and_then(|v| v.as_i64()).unwrap_or(0) < 24
    })
})
.max_iterations(100);  // safety cap
```

### Cancellation

Cooperative cancellation via `CancellationToken`:

```rust
let token = CancellationToken::new();
let ctx = Context::new(adapter).with_cancel(token.clone());

// From an e-stop handler:
token.cancel();
// Sequence will stop at the next step boundary and return Error::Cancelled
```

### Resource Locking

Serialize access to shared physical resources:

```rust
let pallet_zone = ResourceLock::new("pallet");
let step_a = Locked::new(place_left, pallet_zone.clone());
let step_b = Locked::new(place_right, pallet_zone);
// Safe to run in parallel -- mutex ensures exclusive access
```

### Journal (Crash Recovery)

Record step progress for crash recovery:

```rust
let journal = MemoryJournal::new();
let ctx = Context::new(adapter).with_journal(journal.clone());

// After a crash, reuse the same workflow_id:
// already-completed steps are skipped automatically.
```

Implement the `Journal` trait for persistent storage (database, file, etc).

### Error Discrimination

Per-step control over failure behavior:

```rust
impl Step for MyStep {
    fn error_strategy(&self, error: &Error) -> ErrorStrategy {
        match error {
            Error::TaskFailed { message, .. } if message.contains("non-critical") => {
                ErrorStrategy::Skip       // skip and continue
            }
            Error::TaskFailed { message, .. } if message.contains("fatal") => {
                ErrorStrategy::Escalate   // stop, no compensation
            }
            _ => ErrorStrategy::Compensate  // default: undo completed
        }
    }
    // ...
}
```

### Adapters

Adapters bridge tasks to the outside world. Relentless is
transport-agnostic &mdash; use whatever fits your system.

```rust
#[async_trait]
pub trait Adapter: Send + Sync {
    async fn execute(&self, action: &str, args: &[Value]) -> Result<Value>;
    async fn read(&self, key: &str) -> Result<Value>;
    async fn subscribe(&self, topic: &str) -> Result<mpsc::Receiver<Value>> { ... }
}
```

- `LocalAdapter` for testing (in-memory, no hardware)
- Implement `Adapter` for Zenoh, ROS 2, MQTT, gRPC, direct hardware

### Hooks

Monitor execution in production:

```rust
let hooks = Hooks::new()
    .on_step_start(|name, _ctx| println!("Starting {name}"))
    .on_step_end(|name, _ctx| println!("Completed {name}"))
    .on_step_error(|name, _ctx, err| eprintln!("{name} failed: {err}"))
    .on_compensate(|name, _ctx| println!("Compensating {name}"));

let ctx = Context::new(adapter).with_hooks(hooks);
```

### Shared State

Steps share typed state through the context:

```rust
// Writer step
ctx.set("target_pose", Value::str("x=1.0,y=2.0")).await;

// Reader step
let pose = ctx.get_str("target_pose").await.unwrap();
```

## Error Handling

When a step fails, `Error::SequenceFailed` gives full context:

```rust
match workflow.run(&ctx).await {
    Ok(()) => println!("success"),
    Err(Error::SequenceFailed { failed_step, source, compensation_errors }) => {
        println!("Failed at: {failed_step}");
        println!("Cause: {source}");
        println!("All compensations ok: {}", compensation_errors.is_empty());
    }
    Err(Error::Cancelled) => println!("E-stop triggered"),
    Err(Error::Timeout { step, seconds }) => println!("{step} timed out after {seconds}s"),
    Err(e) => println!("Other: {e}"),
}
```

## Examples

See `examples/` for realistic robotics scenarios:

| Example | Demonstrates |
|:--------|:-------------|
| `pick_and_place` | Basic sequence with compensation |
| `dual_arm` | Parallel execution, resource locking |
| `palletizing_loop` | Loop with condition, max iterations |
| `emergency_stop` | Cancellation token |
| `crash_recovery` | Journal-based crash recovery |
| `inspection_sort` | Branch routing |

```bash
cargo run --example pick_and_place
cargo run --example dual_arm
```

## Design Principles

1. **Library, not framework.** You call relentless. It doesn't restructure your code.
2. **Minimal dependencies.** Only `tokio` and `async-trait`.
3. **Testable by default.** Every workflow runs with `LocalAdapter`, no hardware needed.
4. **Transport-agnostic.** Zenoh, ROS 2, MQTT, gRPC &mdash; plug in what you use.
5. **Explicit over magic.** Compensation is declared, not inferred.

## License

MIT. See [LICENSE](LICENSE).
