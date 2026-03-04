//! Crash recovery: use a journal to skip already-completed steps
//! after a restart.
//!
//! Demonstrates: Journal, MemoryJournal, crash recovery.

use relentless::*;

#[tokio::main]
async fn main() {
    let adapter = LocalAdapter::new();
    let journal = MemoryJournal::new();

    // --- First run: completes step_a, then "crashes" on step_b ---
    println!("=== First run (will fail at step_b) ===");

    let ctx = Context::new(adapter.clone()).with_journal(journal.clone());
    let wf_id = ctx.workflow_id.clone();

    let seq = Sequence::new("assembly")
        .step(FnTask::new("step_a", |ctx| {
            Box::pin(async move {
                println!("  step_a: running");
                ctx.execute("step_a", &[]).await?;
                Ok(())
            })
        }))
        .step(FnTask::new("step_b", |_ctx| {
            Box::pin(async move {
                println!("  step_b: simulating crash");
                Err(Error::task_failed("step_b", "power loss"))
            })
        }));

    let _ = seq.run(&ctx).await;

    println!("\nJournal entries:");
    for entry in journal.entries().await {
        println!("  {} {:?}", entry.step, entry.kind);
    }

    // --- Second run: step_a is skipped (already completed) ---
    println!("\n=== Second run (recovery, same workflow_id) ===");

    let mut ctx2 = Context::new(adapter.clone()).with_journal(journal.clone());
    ctx2.workflow_id = wf_id; // reuse the same workflow ID

    let seq2 = Sequence::new("assembly")
        .step(FnTask::new("step_a", |ctx| {
            Box::pin(async move {
                println!("  step_a: running (should be skipped!)");
                ctx.execute("step_a_again", &[]).await?;
                Ok(())
            })
        }))
        .step(FnTask::new("step_b", |ctx| {
            Box::pin(async move {
                println!("  step_b: running (retry succeeds)");
                ctx.execute("step_b", &[]).await?;
                Ok(())
            })
        }));

    match seq2.run(&ctx2).await {
        Ok(()) => println!("Recovery complete!"),
        Err(e) => println!("Failed: {e}"),
    }

    println!("\nActions executed across both runs:");
    for (action, _) in adapter.actions().await {
        println!("  {action}");
    }
}
