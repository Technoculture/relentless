//! Inspection and sorting: classify parts and route to different bins.
//!
//! Demonstrates: Branch, Guard, error discrimination (Skip).

use relentless::*;

#[tokio::main]
async fn main() {
    let adapter = LocalAdapter::new();
    adapter.set("part_grade", Value::str("B")).await;

    let ctx = Context::new(adapter.clone());

    let classify = FnTask::new("classify", |ctx| {
        Box::pin(async move {
            let grade = ctx.read("part_grade").await?;
            println!("  classified part as grade {grade}");
            ctx.set("grade", grade).await;
            Ok(())
        })
    });

    let route = Branch::new("route_to_bin", |ctx| {
        Box::pin(async move {
            let grade = ctx.get_str("grade").await.unwrap_or_default();
            match grade.as_str() {
                "A" => 0,
                "B" => 1,
                _ => 2,
            }
        })
    })
    .branch(FnTask::new("bin_A", |ctx| {
        Box::pin(async move {
            println!("  placing in premium bin A");
            ctx.execute("place", &[Value::str("bin_A")]).await?;
            Ok(())
        })
    }))
    .branch(FnTask::new("bin_B", |ctx| {
        Box::pin(async move {
            println!("  placing in standard bin B");
            ctx.execute("place", &[Value::str("bin_B")]).await?;
            Ok(())
        })
    }))
    .branch(FnTask::new("reject", |ctx| {
        Box::pin(async move {
            println!("  rejecting part");
            ctx.execute("place", &[Value::str("reject")]).await?;
            Ok(())
        })
    }));

    let workflow = Sequence::new("inspect_and_sort")
        .step(classify)
        .step(route);

    println!("Running inspection sort...");
    match workflow.run(&ctx).await {
        Ok(()) => println!("Sort complete!"),
        Err(e) => println!("Failed: {e}"),
    }

    println!("\nActions:");
    for (action, args) in adapter.actions().await {
        let arg_str: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        println!("  {action}({})", arg_str.join(", "));
    }
}
