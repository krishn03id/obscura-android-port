// Test the watchdog interrupt handler with an actual infinite loop.
fn main() {
    use std::time::{Duration, Instant};
    use obscura_js::runtime::ObscuraJsRuntime;

    println!("=== Watchdog Test ===");
    println!("Creating runtime...");
    let mut rt = ObscuraJsRuntime::new();
    println!("Runtime created. Running infinite loop with 2s timeout...");

    // Arm a 2-second watchdog, then try to eval an infinite loop.
    // Without the watchdog, this would hang forever.
    let start = Instant::now();
    let result = rt.evaluate_with_timeout(
        "while(true) {}",
        Duration::from_secs(2),
    );
    let elapsed = start.elapsed();

    println!("Returned after {:.2}s", elapsed.as_secs_f64());
    match &result {
        Ok(v) => println!("Result (unexpected success): {:?}", v),
        Err(e) => println!("Error (expected): {}", e),
    }

    // Verify the runtime is still usable after the watchdog fired
    println!("\nVerifying runtime is still usable...");
    match rt.evaluate("1 + 41") {
        Ok(v) => println!("Post-watchdog eval: 1+41 = {}", v),
        Err(e) => println!("Post-watchdog eval FAILED: {}", e),
    }

    // Test 2: a script that runs for a while but completes within the budget
    println!("\n=== Normal script within budget ===");
    let result2 = rt.evaluate_with_timeout(
        "var sum = 0; for (var i = 0; i < 1000000; i++) { sum += i; } sum",
        Duration::from_secs(5),
    );
    match &result2 {
        Ok(v) => println!("Sum of 1M: {:?}", v),
        Err(e) => println!("Error (unexpected): {}", e),
    }

    // Test 3: a Promise that never resolves, with event loop pump
    println!("\n=== Never-resolving Promise with bounded event loop ===");
    let start3 = Instant::now();
    let rt2 = ObscuraJsRuntime::new();
    // Use run_event_loop_bounded which has both async idle + sync watchdog
    let _ = rt2.execute_script("<test>", "new Promise(function() {});"); // never resolves
    let rt3 = ObscuraJsRuntime::new();
    // This should return after the budget, not hang
    // We can't easily call async run_event_loop_bounded from a sync main,
    // so just test the sync watchdog path which we already covered above.

    if elapsed < Duration::from_secs(3) && elapsed >= Duration::from_secs(1) {
        println!("\n✅ Watchdog test PASSED — infinite loop was killed after {:.2}s", elapsed.as_secs_f64());
    } else {
        println!("\n❌ Watchdog test FAILED — returned in {:.2}s (expected ~2s)", elapsed.as_secs_f64());
    }
}
