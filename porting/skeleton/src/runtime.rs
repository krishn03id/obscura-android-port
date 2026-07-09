//! runtime.rs — rquickjs replacement for the deno_core JsRuntime lifecycle.
//!
//! STATUS: skeleton showing the shape. The real runtime.rs is 2660 lines; port incrementally
//! following GUIDE.md §3.5. This gets bootstrap.js loaded and a script evaluated.

use std::sync::{Arc, Mutex};
use rquickjs::{Runtime, Context};

use crate::bridge::install_bridge;
use crate::state::ObscuraState;

/// bootstrap.js is embedded at compile time (was a V8 snapshot in the original; QuickJS has
/// no snapshot, so we eval it at startup — see FINDINGS.md §3 / deno_core-to-rquickjs.md).
const BOOTSTRAP_JS: &str = include_str!("../js/bootstrap.js");

pub struct ObscuraRuntime {
    pub rt: Runtime,
    pub ctx: Context,
    pub state: Arc<Mutex<ObscuraState>>,
}

impl ObscuraRuntime {
    pub fn new() -> rquickjs::Result<Self> {
        let rt = Runtime::new()?;
        rt.set_max_stack_size(1024 * 1024); // QuickJS default stack is small; bootstrap needs more
        let ctx = Context::full(&rt)?;
        let state = Arc::new(Mutex::new(ObscuraState::default()));

        ctx.with(|ctx| -> rquickjs::Result<()> {
            install_bridge(&ctx, state.clone())?;   // globalThis.Deno.core.ops.*
            ctx.eval::<(), _>(BOOTSTRAP_JS)?;        // builds window/document/console/fetch
            Ok(())
        })?;

        Ok(Self { rt, ctx, state })
    }

    /// Execute a page/user script and pump the (sync) job queue.
    pub fn exec(&self, code: &str) -> rquickjs::Result<()> {
        self.ctx.with(|ctx| ctx.eval::<(), _>(code))?;
        while self.rt.is_job_pending() {
            if let Err(job_err) = self.rt.execute_pending_job() {
                return Err(job_err.0.with(|ctx| {
                    let caught = ctx.catch();
                    rquickjs::Error::new_from_js_message(
                        "job", "exception",
                        format!("pending job threw: {caught:?}"),
                    )
                }));
            }
        }
        Ok(())
    }

    // TODO: watchdog via rt.set_interrupt_handler(...) (see deno_core-to-rquickjs.md).
    // TODO: async variant (AsyncRuntime/AsyncContext) for op_fetch_url + real event loop.
}
