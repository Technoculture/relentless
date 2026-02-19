use std::sync::Arc;
use std::time::Duration;

use relentless::*;

// ── helpers ────────────────────────────────────────────────────────

fn noop_task(name: &str) -> FnTask {
    let n = name.to_string();
    FnTask::new(n, |_ctx| Box::pin(async { Ok(()) }))
}

fn fail_task(name: &str, msg: &str) -> FnTask {
    let n = name.to_string();
    let m = msg.to_string();
    FnTask::new(n.clone(), move |_ctx| {
        let n = n.clone();
        let m = m.clone();
        Box::pin(async move { Err(Error::task_failed(&n, &m)) })
    })
}

fn recording_task(name: &str, action: &str) -> FnTask {
    let action = action.to_string();
    let action2 = action.clone();
    FnTask::new(name, move |ctx| {
        let a = action.clone();
        Box::pin(async move {
            ctx.execute(&a, &[]).await?;
            Ok(())
        })
    })
    .with_compensate(move |ctx| {
        let a = action2.clone();
        Box::pin(async move {
            ctx.execute(&format!("undo_{a}"), &[]).await?;
            Ok(())
        })
    })
}

fn ctx() -> Context {
    Context::new(LocalAdapter::new())
}

fn ctx_with_adapter(adapter: Arc<LocalAdapter>) -> Context {
    Context::new(adapter)
}

// ── sequence tests ─────────────────────────────────────────────────

#[tokio::test]
async fn sequence_runs_all_steps() {
    let adapter = LocalAdapter::new();
    let c = ctx_with_adapter(adapter.clone());

    let seq = Sequence::new("test")
        .step(recording_task("a", "pick"))
        .step(recording_task("b", "place"));

    seq.run(&c).await.unwrap();

    let actions = adapter.actions().await;
    assert_eq!(actions.len(), 2);
    assert_eq!(actions[0].0, "pick");
    assert_eq!(actions[1].0, "place");
}

#[tokio::test]
async fn sequence_compensates_on_failure() {
    let adapter = LocalAdapter::new();
    let c = ctx_with_adapter(adapter.clone());

    let seq = Sequence::new("test")
        .step(recording_task("a", "step_a"))
        .step(recording_task("b", "step_b"))
        .step(fail_task("c", "boom"));

    let err = seq.run(&c).await.unwrap_err();
    assert!(matches!(err, Error::SequenceFailed { .. }));

    let actions = adapter.actions().await;
    let names: Vec<&str> = actions.iter().map(|a| a.0.as_str()).collect();
    // a, b run; c fails; then b, a compensated in reverse
    assert_eq!(names, vec!["step_a", "step_b", "undo_step_b", "undo_step_a"]);
}

#[tokio::test]
async fn sequence_fully_compensated() {
    let c = ctx();
    let seq = Sequence::new("test")
        .step(noop_task("a"))
        .step(fail_task("b", "fail"));

    let err = seq.run(&c).await.unwrap_err();
    assert!(err.fully_compensated());
}

#[tokio::test]
async fn sequence_timeout() {
    let c = ctx();
    let slow = FnTask::new("slow", |_ctx| {
        Box::pin(async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Ok(())
        })
    });

    let seq = Sequence::new("timed")
        .step(slow)
        .timeout(Duration::from_millis(50));

    let err = seq.run(&c).await.unwrap_err();
    assert!(matches!(err, Error::Timeout { .. }));
}

#[tokio::test]
async fn sequence_with_retry() {
    let adapter = LocalAdapter::new();
    let c = ctx_with_adapter(adapter.clone());

    let attempt = Arc::new(tokio::sync::Mutex::new(0u32));
    let attempt2 = attempt.clone();

    let flaky = FnTask::new("flaky", move |ctx| {
        let attempt = attempt2.clone();
        Box::pin(async move {
            let mut n = attempt.lock().await;
            *n += 1;
            if *n < 3 {
                Err(Error::task_failed("flaky", "not yet"))
            } else {
                ctx.execute("success", &[]).await?;
                Ok(())
            }
        })
    });

    let seq = Sequence::new("retry_test")
        .step(flaky)
        .retry(RetryPolicy::exponential(5, Duration::from_millis(1)));

    seq.run(&c).await.unwrap();

    let actions = adapter.actions().await;
    assert_eq!(actions[0].0, "success");
}

// ── cancellation ───────────────────────────────────────────────────

#[tokio::test]
async fn cancellation_stops_sequence() {
    let token = CancellationToken::new();
    let c = ctx().with_cancel(token.clone());

    token.cancel();

    let seq = Sequence::new("test").step(noop_task("a"));
    let err = seq.run(&c).await.unwrap_err();
    assert!(err.is_cancelled());
}

// ── parallel ───────────────────────────────────────────────────────

#[tokio::test]
async fn parallel_runs_all() {
    let adapter = LocalAdapter::new();
    let c = ctx_with_adapter(adapter.clone());

    let par = Parallel::new("par")
        .step(recording_task("a", "left"))
        .step(recording_task("b", "right"));

    par.run(&c).await.unwrap();

    let actions = adapter.actions().await;
    assert_eq!(actions.len(), 2);
}

#[tokio::test]
async fn parallel_compensates_on_failure() {
    let adapter = LocalAdapter::new();
    let c = ctx_with_adapter(adapter.clone());

    let par = Parallel::new("par")
        .step(recording_task("a", "left"))
        .step(fail_task("b", "fail"));

    let err = par.run(&c).await.unwrap_err();
    assert!(matches!(err, Error::ParallelFailed { .. }));

    // "left" succeeded, so it should be compensated
    let actions = adapter.actions().await;
    let names: Vec<&str> = actions.iter().map(|a| a.0.as_str()).collect();
    assert!(names.contains(&"undo_left"));
}

// ── guard ──────────────────────────────────────────────────────────

#[tokio::test]
async fn guard_runs_when_true() {
    let adapter = LocalAdapter::new();
    let c = ctx_with_adapter(adapter.clone());
    c.set("go", Value::Bool(true)).await;

    let guarded = Guard::new(
        "check",
        recording_task("inner", "action"),
        |ctx| Box::pin(async move { ctx.get_bool("go").await }),
    );

    guarded.run(&c).await.unwrap();
    assert_eq!(adapter.actions().await.len(), 1);
}

#[tokio::test]
async fn guard_fails_when_false() {
    let c = ctx();
    c.set("go", Value::Bool(false)).await;

    let guarded = Guard::new("check", noop_task("inner"), |ctx| {
        Box::pin(async move { ctx.get_bool("go").await })
    });

    let err = guarded.run(&c).await.unwrap_err();
    assert!(matches!(err, Error::GuardFailed { .. }));
}

#[tokio::test]
async fn guard_uses_fallback() {
    let adapter = LocalAdapter::new();
    let c = ctx_with_adapter(adapter.clone());
    c.set("go", Value::Bool(false)).await;

    let guarded = Guard::new("check", noop_task("primary"), |ctx| {
        Box::pin(async move { ctx.get_bool("go").await })
    })
    .with_fallback(recording_task("fallback", "fallback_action"));

    guarded.run(&c).await.unwrap();
    let actions = adapter.actions().await;
    assert_eq!(actions[0].0, "fallback_action");
}

// ── branch ─────────────────────────────────────────────────────────

#[tokio::test]
async fn branch_selects_correct_step() {
    let adapter = LocalAdapter::new();
    let c = ctx_with_adapter(adapter.clone());
    c.set("choice", Value::I64(1)).await;

    let branch = Branch::new("route", |ctx| {
        Box::pin(async move {
            ctx.get("choice").await.and_then(|v| v.as_i64()).unwrap_or(0) as usize
        })
    })
    .branch(recording_task("a", "path_a"))
    .branch(recording_task("b", "path_b"));

    branch.run(&c).await.unwrap();
    let actions = adapter.actions().await;
    assert_eq!(actions[0].0, "path_b");
}

// ── loop ───────────────────────────────────────────────────────────

#[tokio::test]
async fn loop_repeats_while_condition() {
    let adapter = LocalAdapter::new();
    let c = ctx_with_adapter(adapter.clone());
    c.set("count", Value::I64(0)).await;

    let body = FnTask::new("increment", |ctx| {
        Box::pin(async move {
            let n = ctx.get("count").await.and_then(|v| v.as_i64()).unwrap_or(0);
            ctx.set("count", Value::I64(n + 1)).await;
            ctx.execute("tick", &[]).await?;
            Ok(())
        })
    });

    let lp = Loop::new("count_to_3", body, |ctx| {
        Box::pin(async move {
            let n = ctx.get("count").await.and_then(|v| v.as_i64()).unwrap_or(0);
            n < 3
        })
    });

    lp.run(&c).await.unwrap();

    let count = c.get("count").await.unwrap().as_i64().unwrap();
    assert_eq!(count, 3);
    assert_eq!(adapter.actions().await.len(), 3);
}

#[tokio::test]
async fn loop_max_iterations() {
    let c = ctx();
    c.set("go", Value::Bool(true)).await;

    let body = noop_task("body");
    let lp = Loop::new("infinite", body, |ctx| {
        Box::pin(async move { ctx.get_bool("go").await })
    })
    .max_iterations(5);

    lp.run(&c).await.unwrap(); // would hang without max_iterations
}

// ── resource lock ──────────────────────────────────────────────────

#[tokio::test]
async fn resource_lock_serializes_access() {
    let adapter = LocalAdapter::new();
    let c = ctx_with_adapter(adapter.clone());

    let lock = ResourceLock::new("gripper");

    let a = Locked::new(recording_task("a", "use_gripper"), lock.clone());
    let b = Locked::new(recording_task("b", "use_gripper"), lock);

    // Run them in a parallel; they should both complete (not deadlock)
    let par = Parallel::new("locked_par").step(a).step(b);
    par.run(&c).await.unwrap();

    assert_eq!(adapter.actions().await.len(), 2);
}

// ── journal ────────────────────────────────────────────────────────

#[tokio::test]
async fn journal_records_steps() {
    let journal = MemoryJournal::new();
    let c = ctx().with_journal(journal.clone());

    let seq = Sequence::new("test")
        .step(noop_task("a"))
        .step(noop_task("b"));

    seq.run(&c).await.unwrap();

    let entries = journal.entries().await;
    // Each step: started + completed = 4 entries
    assert_eq!(entries.len(), 4);
    assert_eq!(entries[0].kind, EntryKind::Started);
    assert_eq!(entries[1].kind, EntryKind::Completed);
}

#[tokio::test]
async fn journal_records_failure() {
    let journal = MemoryJournal::new();
    let c = ctx().with_journal(journal.clone());

    let seq = Sequence::new("test")
        .step(noop_task("a"))
        .step(fail_task("b", "boom"));

    let _ = seq.run(&c).await;

    let entries = journal.entries().await;
    let has_failed = entries.iter().any(|e| matches!(e.kind, EntryKind::Failed(_)));
    let has_compensated = entries.iter().any(|e| e.kind == EntryKind::Compensated);
    assert!(has_failed);
    assert!(has_compensated);
}

// ── hooks ──────────────────────────────────────────────────────────

#[tokio::test]
async fn hooks_fire_on_step_events() {
    use std::sync::Mutex as StdMutex;

    let started = Arc::new(StdMutex::new(Vec::<String>::new()));
    let ended = Arc::new(StdMutex::new(Vec::<String>::new()));

    let s = started.clone();
    let e = ended.clone();

    let hooks = Hooks::new()
        .on_step_start(move |name, _ctx| {
            s.lock().unwrap().push(name.to_string());
        })
        .on_step_end(move |name, _ctx| {
            e.lock().unwrap().push(name.to_string());
        });

    let c = ctx().with_hooks(hooks);

    let seq = Sequence::new("test")
        .step(noop_task("a"))
        .step(noop_task("b"));

    seq.run(&c).await.unwrap();

    assert_eq!(*started.lock().unwrap(), vec!["a", "b"]);
    assert_eq!(*ended.lock().unwrap(), vec!["a", "b"]);
}

// ── error discrimination ───────────────────────────────────────────

#[tokio::test]
async fn skip_strategy_continues_after_failure() {
    use async_trait::async_trait;

    struct SkipOnFail;

    #[async_trait]
    impl Step for SkipOnFail {
        fn name(&self) -> &str { "skip_me" }
        async fn run(&self, _ctx: &Context) -> Result<()> {
            Err(Error::task_failed("skip_me", "non-critical"))
        }
        fn error_strategy(&self, _error: &Error) -> ErrorStrategy {
            ErrorStrategy::Skip
        }
    }

    let adapter = LocalAdapter::new();
    let c = ctx_with_adapter(adapter.clone());

    let seq = Sequence::new("test")
        .step(recording_task("before", "before_action"))
        .step(SkipOnFail)
        .step(recording_task("after", "after_action"));

    seq.run(&c).await.unwrap();

    let actions = adapter.actions().await;
    let names: Vec<&str> = actions.iter().map(|a| a.0.as_str()).collect();
    assert_eq!(names, vec!["before_action", "after_action"]);
}

#[tokio::test]
async fn escalate_strategy_stops_without_compensation() {
    use async_trait::async_trait;

    struct EscalateOnFail;

    #[async_trait]
    impl Step for EscalateOnFail {
        fn name(&self) -> &str { "escalate_me" }
        async fn run(&self, _ctx: &Context) -> Result<()> {
            Err(Error::task_failed("escalate_me", "critical"))
        }
        fn error_strategy(&self, _error: &Error) -> ErrorStrategy {
            ErrorStrategy::Escalate
        }
    }

    let adapter = LocalAdapter::new();
    let c = ctx_with_adapter(adapter.clone());

    let seq = Sequence::new("test")
        .step(recording_task("a", "step_a"))
        .step(EscalateOnFail);

    let err = seq.run(&c).await.unwrap_err();
    match &err {
        Error::SequenceFailed { compensation_errors, .. } => {
            // Escalate means no compensation attempted
            assert!(compensation_errors.is_empty());
        }
        _ => panic!("expected SequenceFailed"),
    }

    // Should NOT have compensation actions
    let actions = adapter.actions().await;
    let names: Vec<&str> = actions.iter().map(|a| a.0.as_str()).collect();
    assert_eq!(names, vec!["step_a"]); // only the run, no undo
}

// ── nesting ────────────────────────────────────────────────────────

#[tokio::test]
async fn nested_sequence_in_parallel() {
    let adapter = LocalAdapter::new();
    let c = ctx_with_adapter(adapter.clone());

    let left = Sequence::new("left")
        .step(recording_task("l1", "left_1"))
        .step(recording_task("l2", "left_2"));

    let right = Sequence::new("right")
        .step(recording_task("r1", "right_1"));

    let par = Parallel::new("both").step(left).step(right);
    par.run(&c).await.unwrap();

    assert_eq!(adapter.actions().await.len(), 3);
}

// ── context shared state ───────────────────────────────────────────

#[tokio::test]
async fn context_state_shared_across_steps() {
    let c = ctx();

    let writer = FnTask::new("writer", |ctx| {
        Box::pin(async move {
            ctx.set("key", Value::str("hello")).await;
            Ok(())
        })
    });

    let reader = FnTask::new("reader", |ctx| {
        Box::pin(async move {
            let val = ctx.get_str("key").await.unwrap();
            assert_eq!(val, "hello");
            Ok(())
        })
    });

    let seq = Sequence::new("test").step(writer).step(reader);
    seq.run(&c).await.unwrap();
}

// ── local adapter ──────────────────────────────────────────────────

#[tokio::test]
async fn local_adapter_read_write() {
    let adapter = LocalAdapter::new();
    adapter.set("sensor", Value::F64(42.0)).await;

    let c = ctx_with_adapter(adapter);
    let val = c.read("sensor").await.unwrap();
    assert_eq!(val.as_f64(), Some(42.0));
}

#[tokio::test]
async fn local_adapter_subscribe() {
    let adapter = LocalAdapter::new();
    let adapter2 = adapter.clone();

    let mut rx = adapter.subscribe("topic").await.unwrap();

    tokio::spawn(async move {
        adapter2.publish("topic", Value::I64(1)).await;
        adapter2.publish("topic", Value::I64(2)).await;
    });

    let v1 = rx.recv().await.unwrap();
    let v2 = rx.recv().await.unwrap();
    assert_eq!(v1.as_i64(), Some(1));
    assert_eq!(v2.as_i64(), Some(2));
}

// ── value ──────────────────────────────────────────────────────────

#[test]
fn value_conversions() {
    assert_eq!(Value::from(true), Value::Bool(true));
    assert_eq!(Value::from(42i64), Value::I64(42));
    assert_eq!(Value::from(3.14f64), Value::F64(3.14));
    assert_eq!(Value::from("hello"), Value::Str("hello".to_string()));
    assert!(Value::None.is_none());
    assert!(!Value::Bool(true).is_none());
}

// ── retry ──────────────────────────────────────────────────────────

#[test]
fn retry_delays() {
    let policy = RetryPolicy::exponential(5, Duration::from_millis(100));
    assert_eq!(policy.delay_for_attempt(1), Duration::ZERO);
    assert_eq!(policy.delay_for_attempt(2), Duration::from_millis(100));
    assert_eq!(policy.delay_for_attempt(3), Duration::from_millis(200));
    assert_eq!(policy.delay_for_attempt(4), Duration::from_millis(400));
}

#[test]
fn retry_max_delay_cap() {
    let policy = RetryPolicy::exponential(10, Duration::from_millis(100))
        .with_max_delay(Duration::from_millis(500));
    assert_eq!(policy.delay_for_attempt(10), Duration::from_millis(500));
}
