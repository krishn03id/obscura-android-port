// Minimal interrupt handler test — just rquickjs, no ObscuraRuntime.
use rquickjs::{Runtime, Context};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn main() {
    println!("=== Minimal Interrupt Handler Test ===");

    let rt = Runtime::new().unwrap();
    rt.set_max_stack_size(1024 * 1024);

    let flag = Arc::new(AtomicBool::new(false));
    let flag2 = flag.clone();

    rt.set_interrupt_handler(Some(Box::new(move || {
        println!("  [interrupt handler called] flag={}", flag2.load(Ordering::SeqCst));
        flag2.load(Ordering::SeqCst)
    })));

    let ctx = Context::full(&rt).unwrap();

    // Test 1: quick eval should work (interrupt handler returns false)
    println!("\nTest 1: quick eval");
    ctx.with(|ctx| {
        let v: i64 = ctx.eval("1 + 1").unwrap();
        println!("  1+1 = {}", v);
    });

    // Test 2: infinite loop with watchdog
    println!("\nTest 2: infinite loop with 2s watchdog");
    let flag3 = flag.clone();
    let handle = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(2));
        println!("  [watchdog] setting flag to true");
        flag3.store(true, Ordering::SeqCst);
    });

    let start = Instant::now();
    let result = ctx.with(|ctx| {
        ctx.eval::<(), _>("while(true) {}")
    });
    let elapsed = start.elapsed();
    let _ = handle.join();

    println!("  Returned after {:.2}s", elapsed.as_secs_f64());
    println!("  Result: {:?}", result);

    // Test 3: runtime still usable?
    println!("\nTest 3: runtime still usable after interrupt?");
    flag.store(false, Ordering::SeqCst);
    ctx.with(|ctx| {
        let v: i64 = ctx.eval("40 + 2").unwrap();
        println!("  40+2 = {}", v);
    });

    if elapsed < Duration::from_secs(4) && elapsed >= Duration::from_millis(1500) {
        println!("\n✅ PASS: infinite loop killed after {:.2}s", elapsed.as_secs_f64());
    } else {
        println!("\n❌ FAIL: returned in {:.2}s (expected ~2s)", elapsed.as_secs_f64());
    }
}
