// Test: call JS_SetInterruptHandler directly via FFI, bypassing rquickjs wrapper.
use rquickjs::{Runtime, Context};
use rquickjs_sys as qjs;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

// Global flag + counter — accessed from the C trampoline
static SHOULD_STOP: AtomicBool = AtomicBool::new(false);
static CALL_COUNT: AtomicU64 = AtomicU64::new(0);

// C-compatible trampoline function
extern "C" fn my_interrupt_handler(
    _rt: *mut qjs::JSRuntime,
    _opaque: *mut std::ffi::c_void,
) -> std::ffi::c_int {
    CALL_COUNT.fetch_add(1, Ordering::SeqCst);
    if SHOULD_STOP.load(Ordering::SeqCst) { 1 } else { 0 }
}

fn main() {
    println!("=== Direct FFI Interrupt Handler Test ===");

    let rt = Runtime::new().unwrap();
    rt.set_max_stack_size(1024 * 1024);

    let ctx = Context::full(&rt).unwrap();

    // Set interrupt handler DIRECTLY via FFI, bypassing rquickjs
    unsafe {
        qjs::JS_SetInterruptHandler(
            rt.inner.lock().as_ref().rt.as_ptr(),
            Some(my_interrupt_handler),
            std::ptr::null_mut(),
        );
    }
    println!("Interrupt handler set via direct FFI");

    // Quick test: handler should be called during eval
    println!("\nTest A: quick eval, checking handler calls");
    let before = CALL_COUNT.load(Ordering::SeqCst);
    ctx.with(|ctx| {
        let v: i64 = ctx.eval("1 + 1").unwrap();
        println!("  1+1 = {}", v);
    });
    let after = CALL_COUNT.load(Ordering::SeqCst);
    println!("  Handler called {} times during quick eval", after - before);

    // Test B: longer loop
    println!("\nTest B: 1M iteration loop");
    let before = CALL_COUNT.load(Ordering::SeqCst);
    ctx.with(|ctx| {
        let v: f64 = ctx.eval("var s=0; for(var i=0;i<1000000;i++){ s++; } s").unwrap();
        println!("  Result: {}", v);
    });
    let after = CALL_COUNT.load(Ordering::SeqCst);
    println!("  Handler called {} times during 1M loop", after - before);

    // Test C: infinite loop with watchdog
    println!("\nTest C: infinite loop with 2s watchdog");
    let handle = std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(2));
        println!("  [watchdog] setting SHOULD_STOP=true");
        SHOULD_STOP.store(true, Ordering::SeqCst);
    });

    let start = Instant::now();
    let result = ctx.with(|ctx| {
        ctx.eval::<(), _>("while(true){}")
    });
    let elapsed = start.elapsed();
    let _ = handle.join();
    let calls = CALL_COUNT.load(Ordering::SeqCst);

    println!("  Returned after {:.2}s", elapsed.as_secs_f64());
    println!("  Handler called {} total times", calls);
    println!("  Result: {:?}", result);

    if elapsed < Duration::from_secs(4) {
        println!("\n✅ PASS: infinite loop killed after {:.2}s", elapsed.as_secs_f64());
    } else {
        println!("\n❌ FAIL: hung for {:.2}s", elapsed.as_secs_f64());
    }
}
