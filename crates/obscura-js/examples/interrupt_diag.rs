// Test if interrupt handler fires at all.
use rquickjs::{Runtime, Context};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn main() {
    println!("=== Interrupt Handler Diagnosis ===");

    let rt = Runtime::new().unwrap();
    rt.set_max_stack_size(1024 * 1024);

    let call_count = Arc::new(AtomicU64::new(0));
    let flag = Arc::new(AtomicBool::new(false));

    let cc2 = call_count.clone();
    let fl2 = flag.clone();
    rt.set_interrupt_handler(Some(Box::new(move || {
        cc2.fetch_add(1, Ordering::SeqCst);
        fl2.load(Ordering::SeqCst)
    })));

    let ctx = Context::full(&rt).unwrap();

    // Test A: Does the handler fire during a loop with function calls?
    println!("\nTest A: loop with function calls (3s budget)");
    let flag3 = flag.clone();
    let cc3 = call_count.clone();
    let handle = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(3));
        println!("  [watchdog] setting flag=true");
        flag3.store(true, Ordering::SeqCst);
    });

    let start = Instant::now();
    let result = ctx.with(|ctx| {
        ctx.eval::<(), _>("function f(){ return 1; } var s=0; while(true){ s += f(); }")
    });
    let elapsed = start.elapsed();
    let _ = handle.join();
    println!("  Returned after {:.2}s", elapsed.as_secs_f64());
    println!("  Handler called {} times", call_count.load(Ordering::SeqCst));
    println!("  Result: {:?}", result);

    // Reset
    flag.store(false, Ordering::SeqCst);
    call_count.store(0, Ordering::SeqCst);

    // Test B: Does the handler fire with a simple tight loop?
    println!("\nTest B: tight while(true){{}} (2s budget)");
    let flag4 = flag.clone();
    let cc4 = call_count.clone();
    let handle2 = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(2));
        println!("  [watchdog] setting flag=true");
        flag4.store(true, Ordering::SeqCst);
    });

    let start2 = Instant::now();
    let result2 = ctx.with(|ctx| {
        ctx.eval::<(), _>("while(true){}")
    });
    let elapsed2 = start2.elapsed();
    let _ = handle2.join();
    println!("  Returned after {:.2}s", elapsed2.as_secs_f64());
    println!("  Handler called {} times", cc4.load(Ordering::SeqCst));
    println!("  Result: {:?}", result2);

    println!("\nDone.");
}
