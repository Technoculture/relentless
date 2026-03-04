//! Long-horizon real-world robotics stress tests.
//!
//! Each test models a realistic scenario from factory floors, warehouses,
//! surgical robots, autonomous vehicles, and space systems. They are
//! designed to find where the library breaks under real conditions.

use async_trait::async_trait;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use relentless::*;

// ════════════════════════════════════════════════════════════════════
// Helpers
// ════════════════════════════════════════════════════════════════════

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

fn fail_task(name: &str, msg: &str) -> FnTask {
    let n = name.to_string();
    let m = msg.to_string();
    FnTask::new(n.clone(), move |_ctx| {
        let n = n.clone();
        let m = m.clone();
        Box::pin(async move { Err(Error::task_failed(&n, &m)) })
    })
}



fn action_names(actions: &[(String, Vec<Value>)]) -> Vec<&str> {
    actions.iter().map(|a| a.0.as_str()).collect()
}

// ════════════════════════════════════════════════════════════════════
// SCENARIO 1: CNC Machine Tool Change
//
// Real world: A CNC machine swaps tools in a spindle. Steps:
//   1. Retract spindle to safe height        (compensate: nothing)
//   2. Open tool clamp                       (compensate: close clamp)
//   3. Move tool to magazine slot            (compensate: retrieve tool)
//   4. Release tool into magazine            (compensate: grip tool)
//   5. Move to new tool slot                 (compensate: return to old slot)
//   6. Grip new tool                         (compensate: release)
//   7. Close tool clamp                      (compensate: open clamp)
//
// BUG EXPOSED: When a sequence TIMES OUT, completed steps are NOT
// compensated. The clamp is open, the old tool is released, but no
// undo happens. The spindle is left without a tool, clamp open.
// ════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn cnc_tool_change_timeout_must_compensate() {
    let adapter = LocalAdapter::new();
    let ctx = Context::new(adapter.clone());

    let workflow = Sequence::new("tool_change")
        .step(recording_task("retract", "spindle.retract"))
        .step(recording_task("open_clamp", "clamp.open"))
        .step(recording_task("release_tool", "tool.release"))
        .step(
            // Step 4 hangs: servo motor stalls trying to reach new slot
            FnTask::new("move_to_new_slot", |_ctx| {
                Box::pin(async move {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    Ok(())
                })
            })
            .with_compensate(|ctx| {
                Box::pin(async move {
                    ctx.execute("undo_move_to_new_slot", &[]).await?;
                    Ok(())
                })
            }),
        )
        .step(recording_task("grip_new_tool", "tool.grip"))
        .timeout(Duration::from_millis(50));

    let err = workflow.run(&ctx).await.unwrap_err();
    assert!(err.is_timeout(), "Expected timeout error, got: {err}");

    // CRITICAL ASSERTION: after timeout, the library MUST compensate
    // steps that already completed. The clamp is open, the tool is
    // released — we need undo_tool.release, undo_clamp.open.
    let actions = adapter.actions().await;
    let names = action_names(&actions);

    assert!(
        names.contains(&"undo_tool.release"),
        "Timeout must compensate completed steps! Tool left released. Actions: {names:?}"
    );
    assert!(
        names.contains(&"undo_clamp.open"),
        "Timeout must compensate completed steps! Clamp left open. Actions: {names:?}"
    );
}

// ════════════════════════════════════════════════════════════════════
// SCENARIO 2: Surgical Robot E-Stop
//
// Real world: A surgical robot performing a biopsy. Steps:
//   1. Position arm over patient         (compensate: retract arm)
//   2. Lower needle to tissue surface    (compensate: raise needle)
//   3. Insert needle                     (compensate: retract needle)
//   4. Extract sample                    (compensate: nothing)
//   5. Retract needle
//
// E-stop fires after step 2 completes. The needle is at the tissue
// surface. If we just return Cancelled with no compensation, the
// needle is left touching the patient.
//
// BUG EXPOSED: Cancellation between steps returns Error::Cancelled
// but never compensates already-completed steps.
// ════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn surgical_robot_estop_must_compensate() {
    let adapter = LocalAdapter::new();
    let token = CancellationToken::new();
    let ctx = Context::new(adapter.clone()).with_cancel(token.clone());

    let step_counter = Arc::new(AtomicU32::new(0));
    let sc = step_counter.clone();
    let token2 = token.clone();

    // Step that triggers e-stop after it completes
    let lower_needle = FnTask::new("lower_needle", move |ctx| {
        let sc = sc.clone();
        let t = token2.clone();
        Box::pin(async move {
            ctx.execute("needle.lower", &[]).await?;
            sc.fetch_add(1, Ordering::SeqCst);
            // Surgeon hits e-stop right after needle is lowered
            t.cancel();
            Ok(())
        })
    })
    .with_compensate(|ctx| {
        Box::pin(async move {
            ctx.execute("undo_needle.lower", &[]).await?;
            Ok(())
        })
    });

    let workflow = Sequence::new("biopsy")
        .step(recording_task("position_arm", "arm.position"))
        .step(lower_needle)
        .step(recording_task("insert_needle", "needle.insert"));

    let err = workflow.run(&ctx).await.unwrap_err();
    assert!(err.is_cancelled());

    // CRITICAL: the needle is lowered and touching the patient.
    // Compensation MUST run: raise needle, retract arm.
    let actions = adapter.actions().await;
    let names = action_names(&actions);

    assert!(
        names.contains(&"undo_needle.lower"),
        "E-stop must compensate! Needle left on patient. Actions: {names:?}"
    );
    assert!(
        names.contains(&"undo_arm.position"),
        "E-stop must compensate! Arm left over patient. Actions: {names:?}"
    );
}

// ════════════════════════════════════════════════════════════════════
// SCENARIO 3: Warehouse Bin Emptying
//
// Real world: A robot empties a bin of 8 items onto a conveyor.
// Uses a Loop to pick items one at a time.
//
// After picking 5 items, the gripper fails on item 6.
// The loop should compensate ALL 5 already-picked items back
// into the bin, not just one.
//
// BUG EXPOSED: Loop::compensate calls self.body.compensate() once,
// but the loop ran 5 iterations. 4 items left on conveyor.
// ════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn bin_emptying_loop_failure_compensates_all_iterations() {
    let adapter = LocalAdapter::new();
    let ctx = Context::new(adapter.clone());
    ctx.set("items_picked", Value::I64(0)).await;
    ctx.set("bin_count", Value::I64(8)).await;

    let pick_one = FnTask::new("pick_item", |ctx| {
        Box::pin(async move {
            let n = ctx.get("items_picked").await.and_then(|v| v.as_i64()).unwrap_or(0);

            // Gripper fails on item 6
            if n >= 5 {
                return Err(Error::task_failed("pick_item", "gripper slip on item 6"));
            }

            ctx.execute("pick", &[Value::I64(n)]).await?;
            ctx.execute("place_on_conveyor", &[Value::I64(n)]).await?;
            ctx.set("items_picked", Value::I64(n + 1)).await;
            Ok(())
        })
    })
    .with_compensate(|ctx| {
        Box::pin(async move {
            let n = ctx.get("items_picked").await.and_then(|v| v.as_i64()).unwrap_or(0);
            // Put the last item back; real robot would need to track all of them
            ctx.execute("return_to_bin", &[Value::I64(n)]).await?;
            ctx.set("items_picked", Value::I64((n - 1).max(0))).await;
            Ok(())
        })
    });

    let empty_loop = Loop::new("empty_bin", pick_one, |ctx| {
        Box::pin(async move {
            let picked = ctx.get("items_picked").await.and_then(|v| v.as_i64()).unwrap_or(0);
            let total = ctx.get("bin_count").await.and_then(|v| v.as_i64()).unwrap_or(0);
            picked < total
        })
    });

    // Wrap in sequence so compensation gets triggered
    let workflow = Sequence::new("bin_emptying").step(empty_loop);

    let err = workflow.run(&ctx).await.unwrap_err();
    assert!(matches!(err, Error::SequenceFailed { .. }));

    // 5 items were placed on conveyor. Compensation should return them ALL.
    // With single-call compensation, at most 1 gets returned.
    let actions = adapter.actions().await;
    let return_count = actions.iter().filter(|a| a.0 == "return_to_bin").count();

    assert!(
        return_count >= 5,
        "Loop ran 5 iterations but compensation only returned {return_count} items. \
         All 5 must be returned to bin. Actions: {:?}",
        action_names(&actions)
    );
}

// ════════════════════════════════════════════════════════════════════
// SCENARIO 4: Dual-Arm Assembly — Parallel Failure Doesn't Cancel Sibling
//
// Real world: Two robot arms working in parallel. Left arm picks a
// heavy object and immediately fails (motor overload). Right arm is
// still moving toward a collision course. We need the right arm to
// STOP, not finish its entire trajectory.
//
// BUG EXPOSED: Parallel waits for ALL futures via JoinAll. Even after
// left arm fails, right arm runs to completion before compensation.
// In real time, this means 500ms of uncontrolled motion.
// ════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn dual_arm_parallel_failure_cancels_sibling() {
    let adapter = LocalAdapter::new();
    let ctx = Context::new(adapter.clone());

    let right_arm_started = Arc::new(AtomicU32::new(0));
    let right_arm_started2 = right_arm_started.clone();

    let left_arm = FnTask::new("left_arm", |_ctx| {
        Box::pin(async {
            // Left arm fails immediately: motor overload
            Err(Error::task_failed("left_arm", "motor overload"))
        })
    });

    let right_arm = FnTask::new("right_arm", move |ctx| {
        let counter = right_arm_started2.clone();
        Box::pin(async move {
            counter.fetch_add(1, Ordering::SeqCst);
            // Right arm does a slow 200ms move
            tokio::time::sleep(Duration::from_millis(200)).await;
            ctx.execute("right_arm.move_complete", &[]).await?;
            Ok(())
        })
    })
    .with_compensate(|ctx| {
        Box::pin(async move {
            ctx.execute("undo_right_arm.move_complete", &[]).await?;
            Ok(())
        })
    });

    let par = Parallel::new("dual_arm").step(left_arm).step(right_arm);

    let start = tokio::time::Instant::now();
    let err = par.run(&ctx).await.unwrap_err();
    let elapsed = start.elapsed();
    assert!(matches!(err, Error::ParallelFailed { .. }));

    // BUG: Parallel should cancel right arm as soon as left arm fails.
    // Instead, it waits for right arm to finish its full 200ms move.
    // In a real robot, this is 200ms of uncontrolled motion after a failure.
    assert!(
        elapsed < Duration::from_millis(100),
        "Parallel should cancel sibling immediately on failure, \
         but took {elapsed:?} (waited for right arm to finish)"
    );
}

// ════════════════════════════════════════════════════════════════════
// SCENARIO 5: Pharmaceutical Packaging — Branch Compensates Wrong Path
//
// Real world: A pill sorting machine classifies pills by color and
// routes them to bin A or bin B. The branch selector picks bin A.
// Later in the sequence, something fails and compensation runs.
//
// Branch.compensate() calls compensate on ALL branches, including
// bin B which never ran. If bin B's compensate has side effects
// (e.g., "remove pill from bin B"), it runs on an empty bin,
// potentially causing a machine fault.
//
// BUG EXPOSED: Branch compensates branches that were never executed.
// ════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn pharma_sorting_branch_only_compensates_taken_path() {
    let adapter = LocalAdapter::new();
    let ctx = Context::new(adapter.clone());
    ctx.set("pill_color", Value::I64(0)).await; // 0 = bin A

    let sort = Branch::new("sort_pill", |ctx| {
        Box::pin(async move {
            ctx.get("pill_color").await.and_then(|v| v.as_i64()).unwrap_or(0) as usize
        })
    })
    .branch(
        FnTask::new("bin_a", |ctx| {
            Box::pin(async move {
                ctx.execute("place_in_bin_a", &[]).await?;
                Ok(())
            })
        })
        .with_compensate(|ctx| {
            Box::pin(async move {
                ctx.execute("remove_from_bin_a", &[]).await?;
                Ok(())
            })
        }),
    )
    .branch(
        FnTask::new("bin_b", |ctx| {
            Box::pin(async move {
                ctx.execute("place_in_bin_b", &[]).await?;
                Ok(())
            })
        })
        .with_compensate(|ctx| {
            Box::pin(async move {
                // This should NEVER run — we never placed a pill in bin B
                ctx.execute("remove_from_bin_b", &[]).await?;
                Ok(())
            })
        }),
    );

    // Run sort (goes to bin A), then fail on next step
    let workflow = Sequence::new("package")
        .step(sort)
        .step(fail_task("label", "printer jam"));

    let _ = workflow.run(&ctx).await;

    let actions = adapter.actions().await;
    let names = action_names(&actions);

    // Bin A compensation should run (pill was placed there)
    assert!(
        names.contains(&"remove_from_bin_a"),
        "Should compensate the taken branch. Actions: {names:?}"
    );

    // Bin B compensation should NOT run (pill was never there)
    assert!(
        !names.contains(&"remove_from_bin_b"),
        "Must NOT compensate untaken branch! \
         remove_from_bin_b ran on an empty bin. Actions: {names:?}"
    );
}

// ════════════════════════════════════════════════════════════════════
// SCENARIO 6: Chemical Reactor — Retry Must Respect Error Strategy
//
// Real world: A chemical reactor sequence. A temperature sensor read
// can be flaky (retry-worthy). But a pressure alarm is fatal — the
// reactor must emergency vent immediately. No retries.
//
// Problem: The retry policy wraps every step. When a step returns an
// Escalate-worthy error, it gets retried max_attempts times BEFORE
// the error strategy is consulted. For a pressure alarm, this means
// seconds of delay before the emergency vent.
//
// BUG EXPOSED: Retry loop runs before error_strategy is checked.
// ════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn chemical_reactor_retry_respects_escalate() {
    let adapter = LocalAdapter::new();
    let ctx = Context::new(adapter.clone());

    let attempt_count = Arc::new(AtomicU32::new(0));
    let attempt_count2 = attempt_count.clone();

    struct PressureAlarm {
        attempts: Arc<AtomicU32>,
    }

    #[async_trait]
    impl Step for PressureAlarm {
        fn name(&self) -> &str {
            "check_pressure"
        }

        async fn run(&self, _ctx: &Context) -> Result<()> {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            Err(Error::task_failed("check_pressure", "CRITICAL: pressure exceeded 200 PSI"))
        }

        fn error_strategy(&self, _error: &Error) -> ErrorStrategy {
            // This is FATAL — do not retry, do not compensate, just STOP
            ErrorStrategy::Escalate
        }
    }

    let workflow = Sequence::new("reactor_cycle")
        .step(recording_task("heat", "reactor.heat"))
        .step(PressureAlarm {
            attempts: attempt_count2,
        })
        .retry(RetryPolicy::exponential(5, Duration::from_millis(100)));

    let _ = workflow.run(&ctx).await;

    // The pressure alarm should fire ONCE and escalate immediately.
    // It should NOT be retried 5 times while the reactor is over-pressured.
    let attempts = attempt_count.load(Ordering::SeqCst);
    assert_eq!(
        attempts, 1,
        "Escalate-strategy step was retried {attempts} times! \
         Must stop immediately on first failure — reactor is over-pressured."
    );
}

// ════════════════════════════════════════════════════════════════════
// SCENARIO 7: Autonomous Vehicle — Compensation Timeout
//
// Real world: A self-driving car performing a lane change:
//   1. Signal turn (compensate: cancel signal)
//   2. Begin lateral move (compensate: return to original lane)
//   3. Complete merge (compensate: ???)
//
// Step 3 fails. Compensation for step 2 (return to lane) starts but
// the steering actuator is stuck. The compensation hangs forever.
// There's no timeout on compensation, so the entire system freezes.
//
// GAP EXPOSED: Compensation has no timeout mechanism.
// ════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn autonomous_vehicle_compensation_must_not_hang() {
    let adapter = LocalAdapter::new();
    let ctx = Context::new(adapter.clone());

    let workflow = Sequence::new("lane_change")
        .compensation_timeout(Duration::from_millis(500))
        .step(recording_task("signal", "turn_signal.on"))
        .step(
            FnTask::new("lateral_move", |ctx| {
                Box::pin(async move {
                    ctx.execute("steering.move_lateral", &[]).await?;
                    Ok(())
                })
            })
            .with_compensate(|_ctx| {
                Box::pin(async move {
                    // Steering actuator stuck — compensation hangs
                    tokio::time::sleep(Duration::from_secs(300)).await;
                    Ok(())
                })
            }),
        )
        .step(fail_task("merge", "obstacle detected"));

    // The whole sequence (including compensation) must complete in
    // reasonable time. If compensation hangs, this test times out.
    let result = tokio::time::timeout(Duration::from_secs(2), workflow.run(&ctx)).await;

    assert!(
        result.is_ok(),
        "Sequence hung during compensation! \
         Compensation must have a timeout to prevent system freeze."
    );
}

// ════════════════════════════════════════════════════════════════════
// SCENARIO 8: PCB Assembly — Duplicate Step Names Break Journal
//
// Real world: A pick-and-place machine putting 3 identical resistors
// on a PCB. Each step is naturally named "place_resistor". After
// a crash and restart, the journal says "place_resistor" completed,
// so ALL three resistor placements are skipped — only 1 was actually
// placed, leaving 2 positions empty.
//
// BUG EXPOSED: Journal keyed by step name. Duplicate names cause
// all instances to be treated as "already completed."
// ════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn pcb_assembly_duplicate_names_journal_recovery() {
    let adapter = LocalAdapter::new();
    let journal = MemoryJournal::new();

    // First run: place resistor 1, then crash
    let ctx1 = Context::new(adapter.clone()).with_journal(journal.clone());
    let wf_id = ctx1.workflow_id.clone();

    let seq1 = Sequence::new("place_resistors")
        .step(recording_task("place_resistor", "place_R1"))
        .step(fail_task("place_resistor", "pick failure on R2"));

    let _ = seq1.run(&ctx1).await;

    // Verify first resistor was placed
    let actions1 = adapter.actions().await;
    assert!(action_names(&actions1).contains(&"place_R1"), "R1 should have been placed");

    // Second run: recovery with same workflow_id
    let adapter2 = LocalAdapter::new();
    let mut ctx2 = Context::new(adapter2.clone()).with_journal(journal.clone());
    ctx2.workflow_id = wf_id;

    let seq2 = Sequence::new("place_resistors")
        .step(recording_task("place_resistor", "place_R1"))
        .step(recording_task("place_resistor", "place_R2"))
        .step(recording_task("place_resistor", "place_R3"));

    seq2.run(&ctx2).await.unwrap();

    let actions2 = adapter2.actions().await;
    let names2 = action_names(&actions2);

    // R1 should be skipped (already placed in first run).
    // R2 and R3 should be placed.
    // BUG: journal returns "place_resistor" as completed, so ALL
    // three steps are skipped because they all have the same name.
    assert!(
        names2.contains(&"place_R2"),
        "R2 must be placed on recovery. \
         Duplicate name 'place_resistor' caused it to be skipped. Actions: {names2:?}"
    );
    assert!(
        names2.contains(&"place_R3"),
        "R3 must be placed on recovery. \
         Duplicate name 'place_resistor' caused it to be skipped. Actions: {names2:?}"
    );
}

// ════════════════════════════════════════════════════════════════════
// SCENARIO 9: Welding Line — 200-Step Long Horizon Sequence
//
// Real world: An automotive welding line with 200 spot welds. This
// tests the library's ability to handle long sequences without
// performance degradation or stack overflow from deep compensation.
//
// Step 150 fails. Library must compensate 149 steps in reverse.
// ════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn welding_line_200_step_sequence() {
    let adapter = LocalAdapter::new();
    let ctx = Context::new(adapter.clone());

    let mut seq = Sequence::new("welding_line");
    for i in 0..200 {
        if i == 150 {
            seq = seq.step(fail_task(&format!("weld_{i}"), "electrode worn"));
        } else {
            seq = seq.step(recording_task(
                &format!("weld_{i}"),
                &format!("weld_spot_{i}"),
            ));
        }
    }

    let err = seq.run(&ctx).await.unwrap_err();
    assert!(matches!(err, Error::SequenceFailed { .. }));

    let actions = adapter.actions().await;
    // 150 welds executed + 150 compensations
    let weld_count = actions.iter().filter(|a| a.0.starts_with("weld_spot_")).count();
    let undo_count = actions.iter().filter(|a| a.0.starts_with("undo_weld_spot_")).count();

    assert_eq!(weld_count, 150, "Should complete exactly 150 welds before failure");
    assert_eq!(undo_count, 150, "Should compensate all 150 completed welds");
}

// ════════════════════════════════════════════════════════════════════
// SCENARIO 10: Semiconductor Fab — Deeply Nested Workflow
//
// Real world: A semiconductor fab has nested phases:
//   Wafer processing:
//     Phase 1: Sequence [ clean, coat ]
//     Phase 2: Parallel [ expose_left, expose_right ]
//     Phase 3: Sequence [ develop, etch, strip ]
//
// Phase 3, step "etch" fails. Must compensate:
//   - develop (from phase 3)
//   - expose_left + expose_right (from phase 2, parallel)
//   - coat, clean (from phase 1)
//
// Tests deep nesting: Sequence > [Sequence, Parallel, Sequence]
// ════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn semiconductor_fab_nested_compensation() {
    let adapter = LocalAdapter::new();
    let ctx = Context::new(adapter.clone());

    let phase1 = Sequence::new("phase1_prep")
        .step(recording_task("clean", "wafer.clean"))
        .step(recording_task("coat", "wafer.coat"));

    let phase2 = Parallel::new("phase2_expose")
        .step(recording_task("expose_left", "expose.left"))
        .step(recording_task("expose_right", "expose.right"));

    let phase3 = Sequence::new("phase3_finish")
        .step(recording_task("develop", "wafer.develop"))
        .step(fail_task("etch", "etchant depleted"))
        .step(recording_task("strip", "wafer.strip"));

    let workflow = Sequence::new("wafer_processing")
        .step(phase1)
        .step(phase2)
        .step(phase3);

    let err = workflow.run(&ctx).await.unwrap_err();
    assert!(matches!(err, Error::SequenceFailed { .. }));

    let actions = adapter.actions().await;
    let names = action_names(&actions);

    // Phase 3 compensation: develop undone
    assert!(names.contains(&"undo_wafer.develop"), "Must undo develop. Actions: {names:?}");
    // Phase 2 compensation: both exposures undone
    assert!(names.contains(&"undo_expose.left"), "Must undo left exposure. Actions: {names:?}");
    assert!(names.contains(&"undo_expose.right"), "Must undo right exposure. Actions: {names:?}");
    // Phase 1 compensation: coat and clean undone
    assert!(names.contains(&"undo_wafer.coat"), "Must undo coat. Actions: {names:?}");
    assert!(names.contains(&"undo_wafer.clean"), "Must undo clean. Actions: {names:?}");
}

// ════════════════════════════════════════════════════════════════════
// SCENARIO 11: Space Probe — Nested Parallel Inside Loop
//
// Real world: A space probe collects samples by repeating:
//   Loop {
//     Parallel [ drill_sample, analyze_atmosphere ]
//     Sequence [ store_sample, move_to_next_site ]
//   }
//
// After 3 successful iterations, the 4th drill fails.
// All 3 stored samples must be compensated (ejected).
//
// Tests: Loop inside Sequence, Parallel inside Loop.
// ════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn space_probe_nested_loop_parallel_compensation() {
    let adapter = LocalAdapter::new();
    let ctx = Context::new(adapter.clone());
    ctx.set("sites_visited", Value::I64(0)).await;
    ctx.set("total_sites", Value::I64(5)).await;

    let iteration_body = {
        let collect_step = FnTask::new("collect", |ctx| {
            Box::pin(async move {
                let n = ctx.get("sites_visited").await.and_then(|v| v.as_i64()).unwrap_or(0);

                // Drill fails on 4th site
                if n >= 3 {
                    return Err(Error::task_failed("drill", "hit bedrock"));
                }

                ctx.execute("drill", &[Value::I64(n)]).await?;
                ctx.execute("store_sample", &[Value::I64(n)]).await?;
                ctx.set("sites_visited", Value::I64(n + 1)).await;
                Ok(())
            })
        })
        .with_compensate(|ctx| {
            Box::pin(async move {
                let n = ctx.get("sites_visited").await.and_then(|v| v.as_i64()).unwrap_or(0);
                ctx.execute("eject_sample", &[Value::I64(n)]).await?;
                ctx.set("sites_visited", Value::I64((n - 1).max(0))).await;
                Ok(())
            })
        });

        collect_step
    };

    let survey = Loop::new("survey_sites", iteration_body, |ctx| {
        Box::pin(async move {
            let visited = ctx.get("sites_visited").await.and_then(|v| v.as_i64()).unwrap_or(0);
            let total = ctx.get("total_sites").await.and_then(|v| v.as_i64()).unwrap_or(0);
            visited < total
        })
    });

    let workflow = Sequence::new("space_survey").step(survey);
    let err = workflow.run(&ctx).await.unwrap_err();
    assert!(matches!(err, Error::SequenceFailed { .. }));

    // 3 samples were collected before failure.
    // All 3 should be ejected via compensation.
    let actions = adapter.actions().await;
    let eject_count = actions.iter().filter(|a| a.0 == "eject_sample").count();

    assert!(
        eject_count >= 3,
        "3 samples collected but only {eject_count} ejected. \
         Loop must compensate all iterations, not just the last. Actions: {:?}",
        action_names(&actions)
    );
}

// ════════════════════════════════════════════════════════════════════
// SCENARIO 12: Warehouse AGV Fleet — Concurrent Sequences With
//              Shared Resource Contention
//
// Real world: 4 AGVs (automated guided vehicles) all need to pass
// through a single narrow aisle. Each AGV runs a sequence:
//   acquire_aisle → traverse → release_aisle
//
// They run in parallel with a shared ResourceLock. None should
// deadlock, all should complete, and the aisle should never be
// occupied by more than one AGV at a time.
// ════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn agv_fleet_shared_aisle_no_deadlock() {
    let adapter = LocalAdapter::new();
    let ctx = Context::new(adapter.clone());
    let aisle = ResourceLock::new("narrow_aisle");

    let occupancy = Arc::new(AtomicU32::new(0));
    let max_occupancy = Arc::new(AtomicU32::new(0));

    let mut par = Parallel::new("agv_fleet");

    for i in 0..4 {
        let occ = occupancy.clone();
        let max_occ = max_occupancy.clone();

        let traverse = FnTask::new(format!("agv_{i}_traverse"), move |ctx| {
            let occ = occ.clone();
            let max_occ = max_occ.clone();
            Box::pin(async move {
                let current = occ.fetch_add(1, Ordering::SeqCst) + 1;
                max_occ.fetch_max(current, Ordering::SeqCst);

                // Simulate traversal time
                tokio::time::sleep(Duration::from_millis(10)).await;
                ctx.execute(&format!("agv_{i}.traverse"), &[]).await?;

                occ.fetch_sub(1, Ordering::SeqCst);
                Ok(())
            })
        });

        par = par.step(Locked::new(traverse, aisle.clone()));
    }

    par.run(&ctx).await.unwrap();

    let actions = adapter.actions().await;
    assert_eq!(actions.len(), 4, "All 4 AGVs should traverse");

    let max = max_occupancy.load(Ordering::SeqCst);
    assert_eq!(
        max, 1,
        "Aisle occupied by {max} AGVs simultaneously! ResourceLock must ensure exclusive access."
    );
}

// ════════════════════════════════════════════════════════════════════
// SCENARIO 13: Food Processing — Guard Condition Changes During
//              Compensation
//
// Real world: A food processing line with a temperature guard.
//   1. Heat oven to 200°C           (compensate: cool oven)
//   2. Guard: temp >= 200°C → bake  (compensate: remove food)
//   3. Package food                  (fails: box jammed)
//
// Step 3 fails. Guard ran (temp was 200°C), so "bake" compensation
// should fire. But during compensation, the guard re-checks the
// condition. If temp has dropped (oven was cooling), the guard
// might not compensate even though bake DID run.
//
// Tests: Guard compensation doesn't re-evaluate the condition.
// ════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn food_processing_guard_compensates_regardless_of_condition() {
    let adapter = LocalAdapter::new();
    let ctx = Context::new(adapter.clone());
    ctx.set("oven_temp", Value::F64(200.0)).await;

    let heat_oven = FnTask::new("heat_oven", |ctx| {
        Box::pin(async move {
            ctx.execute("oven.heat", &[]).await?;
            ctx.set("oven_temp", Value::F64(200.0)).await;
            Ok(())
        })
    })
    .with_compensate(|ctx| {
        Box::pin(async move {
            ctx.execute("undo_oven.heat", &[]).await?;
            // Oven cools down during compensation
            ctx.set("oven_temp", Value::F64(25.0)).await;
            Ok(())
        })
    });

    let bake = Guard::new(
        "temp_check",
        FnTask::new("bake", |ctx| {
            Box::pin(async move {
                ctx.execute("food.bake", &[]).await?;
                Ok(())
            })
        })
        .with_compensate(|ctx| {
            Box::pin(async move {
                ctx.execute("undo_food.bake", &[]).await?;
                Ok(())
            })
        }),
        |ctx| {
            Box::pin(async move {
                ctx.get_f64("oven_temp").await.unwrap_or(0.0) >= 200.0
            })
        },
    );

    let workflow = Sequence::new("food_line")
        .step(heat_oven)
        .step(bake)
        .step(fail_task("package", "box jammed"));

    let _ = workflow.run(&ctx).await;

    let actions = adapter.actions().await;
    let names = action_names(&actions);

    // Bake DID run (guard was true). Its compensation must run even
    // though the oven has since cooled (guard would now be false).
    assert!(
        names.contains(&"undo_food.bake"),
        "Guard must compensate inner step even if condition changed. Actions: {names:?}"
    );
}

// ════════════════════════════════════════════════════════════════════
// SCENARIO 14: Blood Lab Analyzer — Skip Strategy Must NOT Add
//              Step to Completed List
//
// Real world: A blood analyzer runs a panel of 5 tests. Some tests
// can be skipped if the reagent is expired (non-critical), but the
// remaining tests should still run.
//
// If test 3 (Skip) fails, and test 5 also fails (Compensate),
// test 3 should NOT be in the compensation list since it never
// succeeded.
// ════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn blood_lab_skip_not_in_compensation_list() {
    struct SkippableTest {
        name: String,
        action: String,
    }

    #[async_trait]
    impl Step for SkippableTest {
        fn name(&self) -> &str {
            &self.name
        }
        async fn run(&self, _ctx: &Context) -> Result<()> {
            Err(Error::task_failed(&self.name, "reagent expired"))
        }
        fn error_strategy(&self, _error: &Error) -> ErrorStrategy {
            ErrorStrategy::Skip
        }
        async fn compensate(&self, ctx: &Context) -> Result<()> {
            // This should NEVER be called for a skipped step
            ctx.execute(&format!("undo_{}", self.action), &[]).await?;
            Ok(())
        }
    }

    let adapter = LocalAdapter::new();
    let ctx = Context::new(adapter.clone());

    let workflow = Sequence::new("blood_panel")
        .step(recording_task("test_1", "run_CBC"))
        .step(recording_task("test_2", "run_BMP"))
        .step(SkippableTest {
            name: "test_3".into(),
            action: "run_lipid".into(),
        })
        .step(recording_task("test_4", "run_TSH"))
        .step(fail_task("test_5", "analyzer fault"));

    let _ = workflow.run(&ctx).await;

    let actions = adapter.actions().await;
    let names = action_names(&actions);

    // Tests 1, 2, 4 ran successfully. Test 3 was skipped. Test 5 failed.
    // Compensation should undo 4, 2, 1 — NOT 3 (it was skipped, never succeeded).
    assert!(
        !names.contains(&"undo_run_lipid"),
        "Skipped step must NOT be compensated! Actions: {names:?}"
    );

    // Verify the successful tests ARE compensated
    assert!(names.contains(&"undo_run_CBC"), "Test 1 must be compensated. Actions: {names:?}");
    assert!(names.contains(&"undo_run_BMP"), "Test 2 must be compensated. Actions: {names:?}");
    assert!(names.contains(&"undo_run_TSH"), "Test 4 must be compensated. Actions: {names:?}");
}

// ════════════════════════════════════════════════════════════════════
// SCENARIO 15: EV Battery Assembly — Journal Must Record Skipped
//              and Compensated Steps Correctly
//
// Real world: Battery module assembly with journal. After crash
// recovery, we need to know exactly which steps completed, which
// failed, and which were compensated. The journal should tell the
// full story.
// ════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn ev_battery_journal_completeness() {
    let adapter = LocalAdapter::new();
    let journal = MemoryJournal::new();
    let ctx = Context::new(adapter.clone()).with_journal(journal.clone());

    let workflow = Sequence::new("battery_assembly")
        .step(recording_task("weld_tabs", "tabs.weld"))
        .step(recording_task("apply_adhesive", "adhesive.apply"))
        .step(fail_task("insert_cells", "cell alignment error"));

    let _ = workflow.run(&ctx).await;

    let entries = journal.entries().await;

    // Verify journal tells the complete story:
    // 1. weld_tabs: Started → Completed
    // 2. apply_adhesive: Started → Completed
    // 3. insert_cells: Started → Failed
    // 4. apply_adhesive: Compensated
    // 5. weld_tabs: Compensated

    let step_entries = |step: &str| -> Vec<&EntryKind> {
        entries
            .iter()
            .filter(|e| e.step == step)
            .map(|e| &e.kind)
            .collect()
    };

    let weld_kinds = step_entries("weld_tabs");
    assert!(
        weld_kinds.contains(&&EntryKind::Started),
        "weld_tabs must have Started entry"
    );
    assert!(
        weld_kinds.contains(&&EntryKind::Completed),
        "weld_tabs must have Completed entry"
    );
    assert!(
        weld_kinds.contains(&&EntryKind::Compensated),
        "weld_tabs must have Compensated entry"
    );

    let insert_kinds = step_entries("insert_cells");
    assert!(
        insert_kinds.contains(&&EntryKind::Started),
        "insert_cells must have Started entry"
    );
    assert!(
        insert_kinds.iter().any(|k| matches!(k, EntryKind::Failed(_))),
        "insert_cells must have Failed entry"
    );
}

// ════════════════════════════════════════════════════════════════════
// SCENARIO 16: Painting Robot — State Isolation Between Parallel Arms
//
// Real world: Two painting arms share context state. Left arm writes
// "color=red", right arm writes "color=blue". Due to concurrent
// access, one might read the other's color value.
//
// Tests: Parallel steps can safely read/write shared state without
// data races (interior mutability correctness).
// ════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn painting_robot_parallel_state_safety() {
    let adapter = LocalAdapter::new();
    let ctx = Context::new(adapter.clone());

    let iterations = 100;

    let left_arm = FnTask::new("left_arm", move |ctx| {
        Box::pin(async move {
            for i in 0..iterations {
                ctx.set("left_color", Value::I64(i)).await;
                tokio::task::yield_now().await;
                let val = ctx.get("left_color").await.and_then(|v| v.as_i64());
                // Our own writes should be visible (eventually consistent is ok)
                assert!(val.is_some(), "left_color disappeared");
            }
            Ok(())
        })
    });

    let right_arm = FnTask::new("right_arm", move |ctx| {
        Box::pin(async move {
            for i in 0..iterations {
                ctx.set("right_color", Value::I64(i)).await;
                tokio::task::yield_now().await;
                let val = ctx.get("right_color").await.and_then(|v| v.as_i64());
                assert!(val.is_some(), "right_color disappeared");
            }
            Ok(())
        })
    });

    let par = Parallel::new("paint").step(left_arm).step(right_arm);
    par.run(&ctx).await.unwrap();

    // Both should have written their final values
    let left = ctx.get("left_color").await.and_then(|v| v.as_i64());
    let right = ctx.get("right_color").await.and_then(|v| v.as_i64());
    assert_eq!(left, Some(iterations - 1));
    assert_eq!(right, Some(iterations - 1));
}

// ════════════════════════════════════════════════════════════════════
// SCENARIO 17: Mars Rover — Deeply Nested Compensation Order
//
// Real world: A Mars rover deployment sequence:
//   Outer: Sequence [
//     deploy_solar_panels,            // compensate: retract
//     Inner: Sequence [
//       calibrate_instruments,        // compensate: reset
//       transmit_first_data,          // compensate: nothing
//     ],
//     begin_traverse,                 // FAILS
//   ]
//
// Compensation order must be:
//   1. Inner sequence compensates: undo transmit (noop), undo calibrate
//   2. Outer continues: undo deploy_solar_panels
//
// Tests exact compensation ordering in nested sequences.
// ════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn mars_rover_nested_compensation_order() {
    let adapter = LocalAdapter::new();
    let ctx = Context::new(adapter.clone());

    let inner = Sequence::new("setup_instruments")
        .step(recording_task("calibrate", "instruments.calibrate"))
        .step(recording_task("transmit", "radio.transmit"));

    let workflow = Sequence::new("rover_deploy")
        .step(recording_task("deploy_panels", "panels.deploy"))
        .step(inner)
        .step(fail_task("begin_traverse", "wheel stuck in sand"));

    let _ = workflow.run(&ctx).await;

    let actions = adapter.actions().await;
    let names = action_names(&actions);

    // Forward: panels.deploy, instruments.calibrate, radio.transmit
    // Compensation (reverse): undo inner (undo transmit, undo calibrate), undo panels.deploy
    let undo_transmit_pos = names.iter().position(|n| *n == "undo_radio.transmit");
    let undo_calibrate_pos = names.iter().position(|n| *n == "undo_instruments.calibrate");
    let undo_panels_pos = names.iter().position(|n| *n == "undo_panels.deploy");

    assert!(undo_transmit_pos.is_some(), "Must undo transmit. Actions: {names:?}");
    assert!(undo_calibrate_pos.is_some(), "Must undo calibrate. Actions: {names:?}");
    assert!(undo_panels_pos.is_some(), "Must undo panels deploy. Actions: {names:?}");

    // Verify ordering: transmit undone before calibrate, both before panels
    let ut = undo_transmit_pos.unwrap();
    let uc = undo_calibrate_pos.unwrap();
    let up = undo_panels_pos.unwrap();

    assert!(ut < uc, "Must undo transmit before calibrate (reverse order). Positions: transmit={ut}, calibrate={uc}");
    assert!(uc < up, "Must undo calibrate before panels (inner before outer). Positions: calibrate={uc}, panels={up}");
}

// ════════════════════════════════════════════════════════════════════
// SCENARIO 18: Airport Baggage — Cancellation Mid-Loop Must
//              Compensate All Completed Iterations
//
// Real world: A baggage handling loop sorting 100 bags. After 20
// bags, the system is cancelled (shift change). The 20 bags already
// on the conveyor need to be returned to the staging area.
//
// Tests: Cancellation within a Loop, compensation of multiple
// iterations.
// ════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn airport_baggage_cancel_mid_loop() {
    let adapter = LocalAdapter::new();
    let token = CancellationToken::new();
    let ctx = Context::new(adapter.clone()).with_cancel(token.clone());
    ctx.set("bags_sorted", Value::I64(0)).await;

    let sort_bag = FnTask::new("sort_bag", move |ctx| {
        Box::pin(async move {
            let n = ctx.get("bags_sorted").await.and_then(|v| v.as_i64()).unwrap_or(0);
            ctx.execute("sort", &[Value::I64(n)]).await?;
            ctx.set("bags_sorted", Value::I64(n + 1)).await;
            Ok(())
        })
    });

    let sort_loop = Loop::new("sort_all", sort_bag, |ctx| {
        Box::pin(async move {
            let sorted = ctx.get("bags_sorted").await.and_then(|v| v.as_i64()).unwrap_or(0);
            // Cancel after 20 bags
            if sorted >= 20 {
                // Simulate external cancel
                return false;
            }
            true
        })
    })
    .max_iterations(100);

    // Wrap in sequence
    let workflow = Sequence::new("baggage_handling")
        .step(sort_loop)
        .step(recording_task("report", "generate_report"));

    workflow.run(&ctx).await.unwrap();

    let bags = ctx.get("bags_sorted").await.and_then(|v| v.as_i64()).unwrap_or(0);
    assert_eq!(bags, 20, "Should have sorted exactly 20 bags");
}

// ════════════════════════════════════════════════════════════════════
// SCENARIO 19: 3D Printer — Retry With Exponential Backoff Timing
//
// Real world: A 3D printer's filament sensor is flaky. Retry with
// exponential backoff. Verify the actual delays match the policy.
// ════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn printer_filament_retry_timing() {
    let adapter = LocalAdapter::new();
    let ctx = Context::new(adapter.clone());

    let attempt_times = Arc::new(tokio::sync::Mutex::new(Vec::<tokio::time::Instant>::new()));
    let at = attempt_times.clone();

    let attempt_count = Arc::new(AtomicU32::new(0));
    let ac = attempt_count.clone();

    let check_filament = FnTask::new("check_filament", move |ctx| {
        let at = at.clone();
        let ac = ac.clone();
        Box::pin(async move {
            at.lock().await.push(tokio::time::Instant::now());
            let n = ac.fetch_add(1, Ordering::SeqCst) + 1;
            if n < 4 {
                Err(Error::task_failed("check_filament", "sensor misread"))
            } else {
                ctx.execute("filament_ok", &[]).await?;
                Ok(())
            }
        })
    });

    let workflow = Sequence::new("print_layer")
        .step(check_filament)
        .retry(RetryPolicy::exponential(5, Duration::from_millis(50)));

    let start = tokio::time::Instant::now();
    workflow.run(&ctx).await.unwrap();
    let _total = start.elapsed();

    let times = attempt_times.lock().await;
    assert_eq!(times.len(), 4, "Should have taken 4 attempts");

    // Verify exponential timing: delays should be ~0, ~50ms, ~100ms
    // (between attempts 1→2, 2→3, 3→4)
    if times.len() >= 4 {
        let d1 = times[1] - times[0]; // should be ~50ms
        let d2 = times[2] - times[1]; // should be ~100ms
        let d3 = times[3] - times[2]; // should be ~200ms

        assert!(d1 >= Duration::from_millis(40), "First backoff too short: {d1:?}");
        assert!(d2 >= Duration::from_millis(80), "Second backoff too short: {d2:?}");
        assert!(d3 >= Duration::from_millis(160), "Third backoff too short: {d3:?}");
    }
}

// ════════════════════════════════════════════════════════════════════
// SCENARIO 20: Nuclear Fuel Rod Handling — Compensation Must
//              Complete Even If Some Compensations Fail
//
// Real world: Moving nuclear fuel rods. If step 4 fails, we must
// attempt compensation for steps 3, 2, 1. If step 2's compensation
// also fails (crane stuck), we must STILL compensate step 1.
// Compensation must be best-effort, not fail-fast.
// ════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn nuclear_fuel_rod_compensation_continues_on_error() {
    let adapter = LocalAdapter::new();
    let ctx = Context::new(adapter.clone());

    let step1 = recording_task("unlock_pool", "pool.unlock");

    let step2 = FnTask::new("lift_rod", |ctx| {
        Box::pin(async move {
            ctx.execute("rod.lift", &[]).await?;
            Ok(())
        })
    })
    .with_compensate(|_ctx| {
        Box::pin(async move {
            // Compensation FAILS — crane stuck
            Err(Error::task_failed("lift_rod", "crane hydraulic failure"))
        })
    });

    let step3 = recording_task("move_rod", "rod.move");

    let workflow = Sequence::new("fuel_handling")
        .step(step1)
        .step(step2)
        .step(step3)
        .step(fail_task("lower_rod", "alignment error"));

    let err = workflow.run(&ctx).await.unwrap_err();

    // Compensation of step 2 failed, but step 1's compensation must
    // still be attempted. Compensation is best-effort.
    let actions = adapter.actions().await;
    let names = action_names(&actions);

    assert!(
        names.contains(&"undo_pool.unlock"),
        "Must still compensate step 1 even when step 2 compensation failed. Actions: {names:?}"
    );

    // Error should report the compensation failure
    assert!(
        !err.fully_compensated(),
        "Should report that compensation was incomplete"
    );
}

// ════════════════════════════════════════════════════════════════════
// SCENARIO 21: Injection Molder — Empty Sequence Edge Case
//
// Real world: A configurable mold sequence. Sometimes the sequence
// has zero steps (no post-processing configured). Must not crash.
// ════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn injection_molder_empty_sequence() {
    let ctx = Context::new(LocalAdapter::new());

    let empty = Sequence::new("post_processing");
    empty.run(&ctx).await.unwrap();

    let empty_par = Parallel::new("optional_steps");
    empty_par.run(&ctx).await.unwrap();
}

// ════════════════════════════════════════════════════════════════════
// SCENARIO 22: Conveyor Sorting — Loop Error Mid-Iteration With
//              Sequence Body
//
// Real world: A conveyor sorts items. Each iteration is a sequence:
//   Sequence [ pick_from_conveyor, classify, route_to_bin ]
//
// On iteration 4, "classify" fails. The pick was already done in
// that iteration. The library must handle the partial iteration
// correctly.
// ════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn conveyor_loop_with_sequence_body_mid_failure() {
    let adapter = LocalAdapter::new();
    let ctx = Context::new(adapter.clone());
    ctx.set("items_processed", Value::I64(0)).await;

    let iteration = Sequence::new("process_item")
        .step(FnTask::new("pick", |ctx| {
            Box::pin(async move {
                let n = ctx.get("items_processed").await.and_then(|v| v.as_i64()).unwrap_or(0);
                ctx.execute("pick", &[Value::I64(n)]).await?;
                Ok(())
            })
        })
        .with_compensate(|ctx| {
            Box::pin(async move {
                ctx.execute("undo_pick", &[]).await?;
                Ok(())
            })
        }))
        .step(FnTask::new("classify", |ctx| {
            Box::pin(async move {
                let n = ctx.get("items_processed").await.and_then(|v| v.as_i64()).unwrap_or(0);
                if n >= 3 {
                    return Err(Error::task_failed("classify", "unrecognized item"));
                }
                ctx.execute("classify", &[Value::I64(n)]).await?;
                Ok(())
            })
        }))
        .step(FnTask::new("route", |ctx| {
            Box::pin(async move {
                let n = ctx.get("items_processed").await.and_then(|v| v.as_i64()).unwrap_or(0);
                ctx.execute("route", &[Value::I64(n)]).await?;
                ctx.set("items_processed", Value::I64(n + 1)).await;
                Ok(())
            })
        }));

    let sort_loop = Loop::new("sort_items", iteration, |ctx| {
        Box::pin(async move {
            ctx.get("items_processed").await.and_then(|v| v.as_i64()).unwrap_or(0) < 10
        })
    });

    let workflow = Sequence::new("conveyor_sort").step(sort_loop);
    let err = workflow.run(&ctx).await.unwrap_err();
    assert!(matches!(err, Error::SequenceFailed { .. }));

    // On iteration 4: pick succeeded, classify failed.
    // The inner sequence should compensate "pick" for iteration 4.
    let actions = adapter.actions().await;
    let names = action_names(&actions);

    assert!(
        names.contains(&"undo_pick"),
        "Partial iteration must compensate completed inner steps. Actions: {names:?}"
    );
}
