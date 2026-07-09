// Test interrupt handler with AsyncRuntime/AsyncContext (what rquickjs tests with).
use rquickjs::{AsyncRuntime, AsyncContext};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() {
    println!("=== Async Interrupt Handler Test ===");

    let rt = AsyncRuntime::new().unwrap();
    rt.set_max_stack_size(1024 * 1024).await;

    let call_count = Arc::new(AtomicU64::new(0));
    let flag = Arc::new(AtomicBool::new(false));

    let cc2 = call_count.clone();
    let fl2 = flag.clone();
    rt.set_interrupt_handler(Some(Box::new(move || {
        cc2.fetch_add(1, Ordering::SeqCst);
        fl2.load(Ordering::SeqCst)
    }))).await;

    let ctx = AsyncContext::full(&rt).await.unwrap();

    // Quick test
    println!("\nTest A: quick eval");
    let v: i64 = rquickjs::async_with!(ctx => |ctx| {
        ctx.eval("1 + 1")
    }).await.unwrap();
    println!("  1+1 = {}", v);
    println!("  Handler called {} times", call_count.load(Ordering::SeqCst));

    // Infinite loop test
    println!("\nTest B: infinite loop with 2s watchdog");
    let flag3 = flag.clone();
    let handle = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(2));
        println!("  [watchdog] setting flag=true");
        flag3.store(true, Ordering::SeqCst);
    });

    let start = Instant::now();
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        rquickjs::async_with!(ctx => |ctx| {
            ctx.eval::<(), _>("while(true){}")
        })
    ).await;
    let elapsed = start.elapsed();
    let _ = handle.join();
    let calls = call_count.load(Ordering::SeqCst);

    println!("  Returned after {:.2}s", elapsed.as_secs_f64());
    println!("  Handler called {} total times", calls);
    println!("  Result: {:?}", result);

    if elapsed < Duration::from_secs(5) {
        println!("\n✅ PASS: infinite loop killed after {:.2}s", elapsed.as_secs_f64());
    } else {
        println!("\n❌ FAIL: hung for {:.2}s", elapsed.as_secs_f64());
    }
}
