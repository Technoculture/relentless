//! Emergency stop: cancel a running workflow from an external signal.
//!
//! Demonstrates: CancellationToken, cooperative cancellation.

use relentless::*;
use std::time::Duration;

#[tokio::main]
async fn main() {
    let adapter = LocalAdapter::new();
    let token = CancellationToken::new();
    let ctx = Context::new(adapter).with_cancel(token.clone());

    // Simulate an e-stop after 150ms
    let estop = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        println!("  *** E-STOP triggered ***");
        estop.cancel();
    });

    let workflow = Sequence::new("long_operation")
        .step(FnTask::new("step_1", |_ctx| {
            Box::pin(async move {
                println!("  step 1: running (100ms)");
                tokio::time::sleep(Duration::from_millis(100)).await;
                Ok(())
            })
        }))
        .step(FnTask::new("step_2", |_ctx| {
            Box::pin(async move {
                println!("  step 2: running (100ms)");
                tokio::time::sleep(Duration::from_millis(100)).await;
                Ok(())
            })
        }))
        .step(FnTask::new("step_3", |_ctx| {
            Box::pin(async move {
                println!("  step 3: should not reach here");
                Ok(())
            })
        }));

    println!("Running with e-stop after 150ms...");
    match workflow.run(&ctx).await {
        Ok(()) => println!("Completed (unexpected)"),
        Err(e) if e.is_cancelled() => println!("Cancelled as expected: {e}"),
        Err(e) => println!("Other error: {e}"),
    }
}
