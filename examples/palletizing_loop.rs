//! Palletizing loop: pick items from a conveyor until the pallet is full.
//!
//! Demonstrates: Loop, max_iterations, shared state, Guard.

use relentless::*;

#[tokio::main]
async fn main() {
    let adapter = LocalAdapter::new();
    adapter.set("pallet_capacity", Value::I64(6)).await;

    let ctx = Context::new(adapter.clone());
    ctx.set("items_placed", Value::I64(0)).await;

    let pick_and_place = FnTask::new("pick_place_item", |ctx| {
        Box::pin(async move {
            let n = ctx.get("items_placed").await.and_then(|v| v.as_i64()).unwrap_or(0);
            println!("  picking and placing item #{}", n + 1);
            ctx.execute("conveyor.pick", &[]).await?;
            ctx.execute("pallet.place", &[Value::I64(n)]).await?;
            ctx.set("items_placed", Value::I64(n + 1)).await;
            Ok(())
        })
    });

    let palletize = Loop::new("fill_pallet", pick_and_place, |ctx| {
        Box::pin(async move {
            let placed = ctx.get("items_placed").await.and_then(|v| v.as_i64()).unwrap_or(0);
            let capacity = ctx.get("pallet_capacity").await.and_then(|v| v.as_i64()).unwrap_or(6);
            placed < capacity
        })
    })
    .max_iterations(100); // safety cap

    let workflow = Sequence::new("palletizing")
        .step(FnTask::new("start_conveyor", |ctx| {
            Box::pin(async move {
                println!("Starting conveyor...");
                ctx.execute("conveyor.start", &[]).await?;
                Ok(())
            })
        }))
        .step(palletize)
        .step(FnTask::new("eject_pallet", |ctx| {
            Box::pin(async move {
                let n = ctx.get("items_placed").await.and_then(|v| v.as_i64()).unwrap_or(0);
                println!("Ejecting pallet with {n} items");
                ctx.execute("pallet.eject", &[]).await?;
                Ok(())
            })
        }));

    match workflow.run(&ctx).await {
        Ok(()) => println!("Palletizing complete!"),
        Err(e) => println!("Failed: {e}"),
    }
}
