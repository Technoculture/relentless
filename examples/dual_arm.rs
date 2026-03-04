//! Dual-arm assembly: two arms work in parallel, then a shared
//! resource (pallet zone) is accessed with a lock.
//!
//! Demonstrates: Parallel, ResourceLock, Locked, nesting.

use relentless::*;

#[tokio::main]
async fn main() {
    let adapter = LocalAdapter::new();
    let ctx = Context::new(adapter.clone());

    let pallet = ResourceLock::new("pallet_zone");

    let left_arm = Sequence::new("left_arm")
        .step(FnTask::new("left_pick", |ctx| {
            Box::pin(async move {
                println!("  [L] picking part A");
                ctx.execute("left.pick", &[Value::str("A")]).await?;
                Ok(())
            })
        }))
        .step(Locked::new(
            FnTask::new("left_place", |ctx| {
                Box::pin(async move {
                    println!("  [L] placing part A on pallet");
                    ctx.execute("left.place", &[Value::str("A")]).await?;
                    Ok(())
                })
            }),
            pallet.clone(),
        ));

    let right_arm = Sequence::new("right_arm")
        .step(FnTask::new("right_pick", |ctx| {
            Box::pin(async move {
                println!("  [R] picking part B");
                ctx.execute("right.pick", &[Value::str("B")]).await?;
                Ok(())
            })
        }))
        .step(Locked::new(
            FnTask::new("right_place", |ctx| {
                Box::pin(async move {
                    println!("  [R] placing part B on pallet");
                    ctx.execute("right.place", &[Value::str("B")]).await?;
                    Ok(())
                })
            }),
            pallet,
        ));

    let workflow = Parallel::new("dual_arm_assembly").step(left_arm).step(right_arm);

    println!("Running dual-arm assembly...");
    match workflow.run(&ctx).await {
        Ok(()) => println!("Assembly complete!"),
        Err(e) => println!("Failed: {e}"),
    }

    println!("\nActions:");
    for (action, _) in adapter.actions().await {
        println!("  {action}");
    }
}
