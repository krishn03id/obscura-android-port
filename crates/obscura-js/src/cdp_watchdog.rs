//! Shared per-command watchdog for the CDP server.
//!
//! In the V8 build this used IsolateHandle::terminate_execution. In the
//! QuickJS build we use rquickjs's interrupt handler: a flag that the engine
//! polls, returning true to abort execution.
//!
//! The CDP dispatcher holds the JS lock around every command, so at most one
//! command's isolate is ever executing at a time. A single long-lived watchdog
//! thread bounds the current command with a deadline.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::runtime::InterruptHandle;

struct Slot {
    deadline: Instant,
    handle: InterruptHandle,
    gen: u64,
    fired: Arc<AtomicBool>,
}

struct Shared {
    state: Mutex<(Option<Slot>, u64)>,
    cv: Condvar,
}

static SHARED: OnceLock<Arc<Shared>> = OnceLock::new();

fn shared() -> &'static Arc<Shared> {
    SHARED.get_or_init(|| {
        let s = Arc::new(Shared {
            state: Mutex::new((None, 0)),
            cv: Condvar::new(),
        });
        let worker = s.clone();
        std::thread::Builder::new()
            .name("cdp-watchdog".into())
            .spawn(move || watchdog_loop(worker))
            .expect("spawn cdp watchdog");
        s
    })
}

fn watchdog_loop(s: Arc<Shared>) {
    let mut guard = s.state.lock().unwrap();
    loop {
        let next_deadline = match &guard.0 {
            None => None,
            Some(slot) => {
                let now = Instant::now();
                if slot.deadline <= now {
                    slot.fired.store(true, Ordering::SeqCst);
                    slot.handle.set_should_stop(true);
                    guard.0 = None;
                    None
                } else {
                    Some(slot.deadline - now)
                }
            }
        };
        guard = match next_deadline {
            None => s.cv.wait(guard).unwrap(),
            Some(dur) => s.cv.wait_timeout(guard, dur).unwrap().0,
        };
    }
}

pub struct Armed {
    gen: u64,
    fired: Arc<AtomicBool>,
}

pub fn arm(handle: InterruptHandle, budget: Duration) -> Armed {
    let s = shared();
    let mut guard = s.state.lock().unwrap();
    guard.1 += 1;
    let gen = guard.1;
    let fired = Arc::new(AtomicBool::new(false));
    guard.0 = Some(Slot {
        deadline: Instant::now() + budget,
        handle,
        gen,
        fired: fired.clone(),
    });
    s.cv.notify_one();
    Armed { gen, fired }
}

pub fn disarm(armed: Armed) -> bool {
    let s = shared();
    let mut guard = s.state.lock().unwrap();
    if guard.0.as_ref().map(|sl| sl.gen) == Some(armed.gen) {
        guard.0 = None;
        s.cv.notify_one();
    }
    armed.fired.load(Ordering::SeqCst)
}
