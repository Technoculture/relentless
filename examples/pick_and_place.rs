//! Basic pick-and-place with automatic compensation.
//!
//! Demonstrates: Sequence, FnTask, compensation, LocalAdapter.

use relentless::*;

#[tokio::main]
async fn main() {
    let adapter = LocalAdapter::new();
    let ctx = Context::new(adapter.clone());

    let workflow = Sequence::new("pick_and_place")
        .step(
            FnTask::new("approach", |ctx| {
                Box::pin(async move {
                    println!("  moving to pick position");
                    ctx.execute("move_to", &[Value::str("pick_pos")]).await?;
                    Ok(())
                })
            })
            .with_compensate(|ctx| {
                Box::pin(async move {
                    println!("  [undo] returning to home");
                    ctx.execute("move_to", &[Value::str("home")]).await?;
                    Ok(())
                })
            }),
        )
        .step(
            FnTask::new("grip", |ctx| {
                Box::pin(async move {
                    println!("  closing gripper");
                    ctx.execute("gripper.close", &[]).await?;
                    Ok(())
                })
            })
            .with_compensate(|ctx| {
                Box::pin(async move {
                    println!("  [undo] opening gripper");
                    ctx.execute("gripper.open", &[]).await?;
                    Ok(())
                })
            }),
        )
        .step(FnTask::new("place", |ctx| {
            Box::pin(async move {
                println!("  moving to place position");
                ctx.execute("move_to", &[Value::str("place_pos")]).await?;
                println!("  opening gripper");
                ctx.execute("gripper.open", &[]).await?;
                Ok(())
            })
        }));

    println!("Running pick-and-place workflow...");
    match workflow.run(&ctx).await {
        Ok(()) => println!("Done!"),
        Err(e) => println!("Failed: {e}"),
    }

    println!("\nActions executed:");
    for (action, _args) in adapter.actions().await {
        println!("  {action}");
    }
}
