//! runtime.rs — rquickjs replacement for the deno_core JsRuntime lifecycle.
//!
//! Preserves the original public API (ObscuraJsRuntime, RemoteObjectInfo, etc.)
//! so obscura-browser/obscura-cdp compile unchanged.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use obscura_dom::DomTree;
use rquickjs::{Context, Runtime};

use crate::ops::{install_ops, StoredNetworkResponseBody};
use crate::state::{ObscuraState, SharedState};

// ---------------------------------------------------------------------------
// Constants and type definitions
// ---------------------------------------------------------------------------

/// bootstrap.js is embedded at compile time (was a V8 snapshot; QuickJS has none).
const BOOTSTRAP_JS: &str = include_str!("../js/bootstrap.js");

/// Handle that can interrupt JS execution. Replaces deno_core's IsolateHandle.
/// The runtime's interrupt handler checks `should_stop`; when true, execution
/// is aborted.
#[derive(Clone)]
pub struct InterruptHandle {
    should_stop: Arc<AtomicBool>,
}

impl InterruptHandle {
    fn new() -> Self {
        InterruptHandle { should_stop: Arc::new(AtomicBool::new(false)) }
    }

    pub fn set_should_stop(&self, v: bool) {
        self.should_stop.store(v, Ordering::SeqCst);
    }

    fn is_stopped(&self) -> bool {
        self.should_stop.load(Ordering::SeqCst)
    }

    pub fn clear(&self) {
        self.should_stop.store(false, Ordering::SeqCst);
    }
}

/// Re-exported so other crates can name the interrupt handle without depending
/// on rquickjs internals.
pub use crate::runtime::InterruptHandle as IsolateHandle;

#[derive(Debug, Clone)]
pub struct RemoteObjectInfo {
    pub js_type: String,
    pub subtype: Option<String>,
    pub class_name: String,
    pub description: String,
    pub object_id: Option<String>,
    pub value: Option<serde_json::Value>,
}

pub struct ObscuraJsRuntime {
    rt: Runtime,
    ctx: Context,
    state: SharedState,
    object_store: HashMap<String, String>,
    object_counter: u64,
    interrupt_handle: InterruptHandle,
}

/// Handle to an armed watchdog (see [`ObscuraJsRuntime::arm_watchdog`]).
pub struct WatchdogToken {
    pair: Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>,
    join: Option<std::thread::JoinHandle<()>>,
    fired: Arc<AtomicBool>,
    interrupt_handle: InterruptHandle,
}

// ---------------------------------------------------------------------------
// Watchdog implementation
// ---------------------------------------------------------------------------

pub fn spawn_watchdog(handle: InterruptHandle, budget: std::time::Duration) -> WatchdogToken {
    let pair = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let fired = Arc::new(AtomicBool::new(false));
    let pair_c = pair.clone();
    let fired_c = fired.clone();
    let handle_c = handle.clone();
    let join = std::thread::spawn(move || {
        let (lock, cvar) = &*pair_c;
        let mut cancelled = lock.lock().unwrap();
        let deadline = std::time::Instant::now() + budget;
        loop {
            if *cancelled { return; }
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                fired_c.store(true, Ordering::SeqCst);
                handle_c.set_should_stop(true);
                return;
            }
            let (guard, _) = cvar.wait_timeout(cancelled, remaining).unwrap();
            cancelled = guard;
            if *cancelled { return; }
        }
    });
    WatchdogToken { pair, join: Some(join), fired, interrupt_handle: handle }
}

impl WatchdogToken {
    pub fn stop(mut self) -> bool {
        {
            let (lock, cvar) = &*self.pair;
            *lock.lock().unwrap() = true;
            cvar.notify_one();
        }
        if let Some(j) = self.join.take() { let _ = j.join(); }
        self.fired.load(Ordering::SeqCst)
    }
}

// ===========================================================================
// ObscuraJsRuntime implementation
// ===========================================================================

impl ObscuraJsRuntime {
    // -----------------------------------------------------------------------
    // Construction
    // -----------------------------------------------------------------------

    pub fn new() -> Self {
        Self::with_base_url("about:blank")
    }

    pub fn with_base_url(base_url: &str) -> Self {
        Self::with_base_url_and_proxy(base_url, None)
    }

    pub fn with_base_url_and_proxy(base_url: &str, proxy_url: Option<String>) -> Self {
        let rt = Runtime::new().expect("create rquickjs runtime");
        rt.set_max_stack_size(1024 * 1024);

        // Register the ES module loader so dynamic import() works
        let module_loader = crate::module_loader::ObscuraModuleLoader::with_proxy(base_url, proxy_url);
        rt.set_loader(module_loader.clone(), module_loader);

        let interrupt_handle = InterruptHandle::new();
        let ih = interrupt_handle.clone();
        rt.set_interrupt_handler(Some(Box::new(move || {
            ih.is_stopped()
        })));

        let ctx = Context::full(&rt).expect("create rquickjs context");
        let state: SharedState = Arc::new(Mutex::new(ObscuraState::new()));

        // Install the Deno.core.ops shim before bootstrap
        let bootstrap_result = ctx.with(|ctx| -> rquickjs::Result<()> {
            install_ops(&ctx, state.clone())?;
            // Store bootstrap source as a global string, then eval it via JS eval()
            // so we can catch the error message properly.
            ctx.globals().set("__bootstrap_src", BOOTSTRAP_JS)?;
            let result: String = ctx.eval(
                r#"(function() {
                    try {
                        eval(globalThis.__bootstrap_src);
                        return 'OK';
                    } catch(e) {
                        return 'ERR:' + (e && e.message ? e.message : String(e)) + (e && e.stack ? '\n@@@' + e.stack : '');
                    }
                })()"#
            )?;
            ctx.eval::<(), _>("delete globalThis.__bootstrap_src")?;
            if result.starts_with("ERR:") {
                return Err(rquickjs::Error::new_from_js_message(
                    "bootstrap", "eval", result[4..].to_string(),
                ));
            }
            Ok(())
        });
        bootstrap_result.expect("bootstrap should not fail");

        // Init globals for object tracking
        ctx.with(|ctx| {
            ctx.eval::<(), _>("globalThis.__obscura_objects = {}; globalThis.__obscura_oid = 0;")
                .ok()
        });

        Self {
            rt,
            ctx,
            state,
            object_store: HashMap::new(),
            object_counter: 0,
            interrupt_handle,
        }
    }

    // -----------------------------------------------------------------------
    // State setters
    // -----------------------------------------------------------------------

    pub fn set_cookie_jar(&self, jar: Arc<obscura_net::CookieJar>) {
        self.state.lock().unwrap().cookie_jar = Some(jar);
    }

    pub fn set_http_client(&self, client: Arc<obscura_net::ObscuraHttpClient>) {
        self.state.lock().unwrap().http_client = Some(client);
    }

    #[cfg(feature = "stealth")]
    pub fn set_stealth_client(&self, client: Arc<obscura_net::StealthHttpClient>) {
        self.state.lock().unwrap().stealth_client = Some(client);
    }

    pub fn set_dom(&self, dom: DomTree) {
        self.state.lock().unwrap().dom = Some(dom);
    }

    pub fn set_url(&self, url: &str) {
        self.state.lock().unwrap().url = url.to_string();
    }

    pub fn set_encoding(&self, encoding: &str) {
        self.state.lock().unwrap().encoding = encoding.to_string();
    }

    pub fn set_title(&self, title: &str) {
        self.state.lock().unwrap().title = title.to_string();
    }

    pub fn set_blocked_urls(&self, patterns: Vec<String>) {
        self.state.lock().unwrap().blocked_urls = patterns;
    }

    pub fn set_intercept_tx(&self, tx: tokio::sync::mpsc::UnboundedSender<crate::ops::InterceptedRequest>) {
        let mut state = self.state.lock().unwrap();
        state.intercept_tx = Some(tx);
    }

    pub fn set_intercept_enabled(&self, enabled: bool) {
        let mut state = self.state.lock().unwrap();
        state.intercept_enabled = enabled;
    }

    pub fn set_user_agent(&mut self, ua: &str) {
        let escaped = ua.replace('\\', "\\\\").replace('\'', "\\'");
        self.exec_eval(&format!("globalThis.__obscura_ua = '{}';", escaped));
    }

    pub fn set_platform(&mut self, platform: &str, ua_platform: &str, ua_platform_version: &str) {
        let p = platform.replace('\'', "\\'");
        let uap = ua_platform.replace('\'', "\\'");
        let uapv = ua_platform_version.replace('\'', "\\'");
        self.exec_eval(&format!(
            "globalThis.__obscura_platform='{}';globalThis.__obscura_ua_platform='{}';globalThis.__obscura_ua_platform_version='{}';",
            p, uap, uapv
        ));
    }

    pub fn set_stealth(&mut self, enabled: bool) {
        self.exec_eval(&format!("globalThis.__obscura_stealth = {};", enabled));
    }

    pub fn run_page_init(&mut self) {
        self.exec_eval("globalThis.__obscura_init();");
    }

    pub fn set_geolocation(&mut self, latitude: f64, longitude: f64) {
        self.exec_eval(&format!(
            "globalThis.__obscura_geo_lat={};globalThis.__obscura_geo_lon={};",
            latitude, longitude
        ));
    }

    // -----------------------------------------------------------------------
    // State getters
    // -----------------------------------------------------------------------

    pub fn take_pending_navigation(&self) -> Option<(String, String, String)> {
        self.state.lock().unwrap().pending_navigation.take()
    }

    pub fn take_pending_binding_calls(&self) -> Vec<(String, String)> {
        std::mem::take(&mut self.state.lock().unwrap().pending_binding_calls)
    }

    pub fn get_network_response_body(&self, request_id: &str) -> Option<StoredNetworkResponseBody> {
        self.state.lock().unwrap().network_response_bodies.get(request_id).cloned()
    }

    pub fn clear_network_response_bodies(&self) {
        let mut state = self.state.lock().unwrap();
        state.network_response_bodies.clear();
        state.network_response_body_order.clear();
    }

    // -----------------------------------------------------------------------
    // Execution helpers
    // -----------------------------------------------------------------------

    /// Execute a script (fire-and-forget). Returns Ok(()) on success.
    fn exec_eval(&mut self, code: &str) {
        let _ = self.ctx.with(|ctx| ctx.eval::<(), _>(code));
    }

    /// Execute a script and return the result as a JSON value.
    fn eval_json(&self, code: &str) -> Result<serde_json::Value, String> {
        let wrapper = format!(
            "(function() {{ var __r = ({}); return typeof __r === 'undefined' ? 'null' : JSON.stringify(__r); }})()",
            code
        );
        let json_str: String = self.ctx.with(|ctx| ctx.eval(wrapper.as_str()))
            .map_err(|e| format!("JS error: {}", e))?;
        if json_str == "null" {
            return Ok(serde_json::Value::Null);
        }
        serde_json::from_str(&json_str).map_err(|e| format!("JSON parse error: {}", e))
    }

    pub fn evaluate(&mut self, expression: &str) -> Result<serde_json::Value, String> {
        let wrapped = Self::wrap_expression(expression);
        self.eval_json(&wrapped)
    }

    pub fn execute_script(&mut self, _name: &str, source: &str) -> Result<(), String> {
        self.ctx.with(|ctx| ctx.eval::<(), _>(source))
            .map_err(|e| format!("JS error: {}", e))?;
        Ok(())
    }

    pub fn execute_script_guarded(&mut self, name: &str, source: &str) -> Result<(), String> {
        if source.len() < 10_000 {
            self.execute_script(name, source)
        } else {
            self.execute_script_with_timeout(source, std::time::Duration::from_secs(5))
        }
    }

    pub fn execute_script_with_timeout(
        &mut self,
        source: &str,
        timeout: std::time::Duration,
    ) -> Result<(), String> {
        if timeout.is_zero() {
            return self.execute_script("<script>", source);
        }
        let token = self.arm_watchdog(timeout);
        let result = self.ctx.with(|ctx| ctx.eval::<(), _>(source));
        let fired = self.disarm_watchdog(token);
        match result {
            Ok(_) if !fired => Ok(()),
            Ok(_) => Ok(()),
            Err(e) => {
                if fired || e.to_string().contains("interrupted") {
                    Ok(())
                } else {
                    Err(format!("JS error: {}", e))
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Event loop
    // -----------------------------------------------------------------------

    pub async fn run_event_loop(&mut self) -> Result<(), String> {
        self.pump_jobs_and_async();
        Ok(())
    }

    /// Pump the rquickjs job queue, process fetch queue, and resolve completed async ops.
    fn pump_jobs_and_async(&self) {
        // First: drain the fetch queue and spawn async tasks
        self.drain_fetch_queue();
        self.drain_search_queue();
        // Resolve any completed async ops
        self.resolve_completed_async();
        // Pump pending jobs
        while self.rt.is_job_pending() {
            if self.rt.execute_pending_job().is_err() {
                break;
            }
        }
        // Resolving promises may have created more jobs or completed more async ops
        let pending = !self.state.lock().unwrap().pending_async_results.is_empty();
        if pending || self.rt.is_job_pending() {
            self.resolve_completed_async();
            while self.rt.is_job_pending() {
                if self.rt.execute_pending_job().is_err() {
                    break;
                }
            }
        }
    }

    // JS→Rust async bridge: QuickJS cannot await Rust futures, so JS pushes
    // fetch/search requests into a global queue and registers resolver
    // callbacks. Rust drains the queue here, spawns tokio tasks that perform
    // the real I/O, and writes results back so resolve_completed_async can
    // invoke the JS resolvers.
    /// Drain the JS fetch queue and spawn async tasks for each request.
    fn drain_fetch_queue(&self) {
        // Drain the queue from JS — get all items as JSON and clear the array
        let queue_json: String = match self.ctx.with(|ctx| {
            ctx.eval::<String, _>(
                r#"(function() {
                    var q = globalThis.__obscura_fetch_queue || [];
                    var items = JSON.stringify(q);
                    globalThis.__obscura_fetch_queue = [];
                    return items;
                })()"#
            )
        }) {
            Ok(s) => s,
            Err(_) => return,
        };

        if queue_json == "[]" {
            // Also resolve any completed async ops
            return;
        }

        // Parse the queue items
        let items: Vec<serde_json::Value> = match serde_json::from_str(&queue_json) {
            Ok(v) => v,
            Err(_) => return,
        };

        // Spawn a tokio task for each fetch request
        for item in items {
            let id = item.get("id").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let url = item.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let method = item.get("method").and_then(|v| v.as_str()).unwrap_or("GET").to_string();
            let headers = item.get("headers").and_then(|v| v.as_str()).unwrap_or("{}").to_string();
            let body = item.get("body").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let origin = item.get("origin").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let mode = item.get("mode").and_then(|v| v.as_str()).unwrap_or("").to_string();

            let st = self.state.clone();
            tokio::spawn(async move {
                let result = crate::ops::op_fetch_url_impl(
                    &st, url, method, headers, body, origin, mode
                ).await;
                st.lock().unwrap().pending_async_results.insert(id, result);
            });
        }
    }

    /// Drain the JS search queue and spawn async tasks for each query.
    fn drain_search_queue(&self) {
        let queue_json: String = match self.ctx.with(|ctx| {
            ctx.eval::<String, _>(
                r#"(function() {
                    var q = globalThis.__obscura_search_queue || [];
                    var items = JSON.stringify(q);
                    globalThis.__obscura_search_queue = [];
                    return items;
                })()"#
            )
        }) {
            Ok(s) => s,
            Err(_) => return,
        };

        if queue_json == "[]" { return; }

        let items: Vec<serde_json::Value> = match serde_json::from_str(&queue_json) {
            Ok(v) => v,
            Err(_) => return,
        };

        for item in items {
            let id = item.get("id").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let query = item.get("query").and_then(|v| v.as_str()).unwrap_or("").to_string();

            let st = self.state.clone();
            tokio::spawn(async move {
                let result = crate::ops::op_web_search_impl(&query).await;
                st.lock().unwrap().pending_async_results.insert(id, result);
            });
        }
    }

    /// Check for completed async ops and resolve their Promises via JS.
    fn resolve_completed_async(&self) {
        let completed: Vec<(usize, Result<String, String>)> = {
            let mut st = self.state.lock().unwrap();
            st.pending_async_results.drain().collect()
        };
        if completed.is_empty() { return; }

        // Each completed op is resolved via whichever resolver map (search or
        // fetch) currently holds its callback. The id is shared across both
        // maps, so checking both covers both async op types — only one will
        // match for any given id.
        let _ = self.ctx.with(|ctx| -> rquickjs::Result<()> {
            for (id, result) in completed {
                match result {
                    Ok(json_val) => {
                        let code = format!(
                            r#"(function() {{
                                if (globalThis.__obscura_search_resolvers && globalThis.__obscura_search_resolvers[{id}]) {{
                                    var r = globalThis.__obscura_search_resolvers[{id}]; delete globalThis.__obscura_search_resolvers[{id}];
                                    r.resolve({val});
                                }} else if (globalThis.__obscura_fetch_resolvers && globalThis.__obscura_fetch_resolvers[{id}]) {{
                                    var r = globalThis.__obscura_fetch_resolvers[{id}]; delete globalThis.__obscura_fetch_resolvers[{id}];
                                    r.resolve({val});
                                }}
                            }})()"#,
                            id = id, val = json_val
                        );
                        let _ = ctx.eval::<(), _>(code.as_str());
                    }
                    Err(err) => {
                        let err_escaped = err.replace('\\', "\\\\").replace('\'', "\\'");
                        let code = format!(
                            r#"(function() {{
                                if (globalThis.__obscura_search_resolvers && globalThis.__obscura_search_resolvers[{id}]) {{
                                    var r = globalThis.__obscura_search_resolvers[{id}]; delete globalThis.__obscura_search_resolvers[{id}];
                                    r.reject(new Error('{err}'));
                                }} else if (globalThis.__obscura_fetch_resolvers && globalThis.__obscura_fetch_resolvers[{id}]) {{
                                    var r = globalThis.__obscura_fetch_resolvers[{id}]; delete globalThis.__obscura_fetch_resolvers[{id}];
                                    r.reject(new Error('{err}'));
                                }}
                            }})()"#,
                            id = id, err = err_escaped
                        );
                        let _ = ctx.eval::<(), _>(code.as_str());
                    }
                }
            }
            Ok(())
        });
    }

    // -----------------------------------------------------------------------
    // Watchdog control
    // -----------------------------------------------------------------------

    pub fn arm_watchdog(&mut self, budget: std::time::Duration) -> WatchdogToken {
        spawn_watchdog(self.interrupt_handle.clone(), budget)
    }

    pub fn disarm_watchdog(&mut self, token: WatchdogToken) -> bool {
        let fired = token.stop();
        if fired {
            self.interrupt_handle.clear();
            tracing::warn!("JS watchdog fired: terminated a synchronous overrun");
        }
        fired
    }

    pub fn isolate_handle(&self) -> InterruptHandle {
        self.interrupt_handle.clone()
    }

    pub fn cancel_termination(&mut self) {
        self.interrupt_handle.clear();
    }

    // -----------------------------------------------------------------------
    // Bounded execution
    // -----------------------------------------------------------------------

    pub async fn run_event_loop_bounded(&mut self, budget_ms: u64) -> Result<(), String> {
        if budget_ms == 0 {
            return self.run_event_loop().await;
        }
        let budget = std::time::Duration::from_millis(budget_ms);
        let token = self.arm_watchdog(budget + std::time::Duration::from_millis(500));
        // Pump in a loop for the budget duration
        let deadline = std::time::Instant::now() + budget;
        loop {
            self.pump_jobs_and_async();
            if std::time::Instant::now() >= deadline {
                break;
            }
            // Brief yield to allow async ops to complete
            tokio::task::yield_now().await;
        }
        let _ = self.disarm_watchdog(token);
        Ok(())
    }

    pub fn evaluate_with_timeout(
        &mut self,
        expression: &str,
        timeout: std::time::Duration,
    ) -> Result<serde_json::Value, String> {
        if timeout.is_zero() {
            return self.evaluate(expression);
        }
        let wrapped = Self::wrap_expression(expression);
        let token = self.arm_watchdog(timeout);
        let result = self.eval_json(&wrapped);
        let fired = self.disarm_watchdog(token);
        match result {
            Ok(v) if !fired => Ok(v),
            Ok(_) => Err("eval timed out".to_string()),
            Err(e) => {
                if fired || e.contains("interrupted") {
                    Err("eval timed out".to_string())
                } else {
                    Err(e)
                }
            }
        }
    }

    pub async fn resolve_promises(&mut self) {
        self.pump_jobs_and_async();
    }

    pub async fn resolve_promises_until<F>(&mut self, mut done_check: F, max_total_ms: u64)
    where
        F: FnMut(&Self) -> bool,
    {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(max_total_ms);
        loop {
            self.pump_jobs_and_async();
            if done_check(self) { return; }
            if std::time::Instant::now() >= deadline { return; }
            tokio::task::yield_now().await;
        }
    }

    // -----------------------------------------------------------------------
    // DOM helpers
    // -----------------------------------------------------------------------

    pub fn take_dom(&self) -> Option<DomTree> {
        self.state.lock().unwrap().dom.take()
    }

    pub fn with_dom<R>(&self, f: impl FnOnce(&DomTree) -> R) -> Option<R> {
        let gs = self.state.lock().unwrap();
        gs.dom.as_ref().map(f)
    }

    pub fn fetched_urls(&self) -> Vec<String> {
        self.state.lock().unwrap().fetched_urls.clone()
    }

    // -----------------------------------------------------------------------
    // CDP evaluation
    // -----------------------------------------------------------------------

    pub async fn evaluate_for_cdp(
        &mut self,
        expression: &str,
        return_by_value: bool,
        await_promise: bool,
    ) -> Result<RemoteObjectInfo, String> {
        if !await_promise && return_by_value {
            let val = self.evaluate(expression)?;
            return Ok(Self::info_from_json(&val));
        }

        self.object_counter += 1;
        let oid = self.make_oid(self.object_counter);
        let cleaned_expr = expression
            .trim()
            .trim_end_matches(|c: char| c == ';' || c.is_whitespace());

        let done_counter = self.object_counter;
        let meta_code = if await_promise {
            format!(
                "(async function() {{\n\
                    try {{\n\
                        var __result = await (\n{expr}\n);\n\
                        globalThis.__obscura_objects['{oid}'] = __result;\n\
                        globalThis.__obscura_await_meta = {meta_fn};\n\
                        globalThis.__obscura_await_rejected = false;\n\
                    }} catch(e) {{\n\
                        globalThis.__obscura_objects['{oid}'] = e;\n\
                        globalThis.__obscura_await_meta = {err_meta_fn};\n\
                        globalThis.__obscura_await_rejected = true;\n\
                    }}\n\
                    globalThis.__obscura_done_{done_counter} = true;\n\
                }})()",
                expr = cleaned_expr,
                oid = oid,
                meta_fn = Self::meta_extract_js("__result"),
                err_meta_fn = Self::meta_extract_js("e"),
                done_counter = done_counter,
            )
        } else {
            format!(
                "(function() {{\n\
                    var __result;\n\
                    try {{ __result = (\n{expr}\n); }} catch(e) {{ __result = undefined; }}\n\
                    globalThis.__obscura_objects['{oid}'] = __result;\n\
                    return {meta_fn};\n\
                }})()",
                expr = cleaned_expr,
                oid = oid,
                meta_fn = Self::meta_extract_js("__result"),
            )
        };

        let meta_result = if await_promise {
            // Eval the async IIFE
            self.ctx.with(|ctx| ctx.eval::<(), _>(meta_code.as_str()))
                .map_err(|e| format!("JS error: {}", e))?;

            // Pump until done
            let sentinel = format!("globalThis.__obscura_done_{} === true", done_counter);
            self.resolve_promises_until(
                |rt| rt.check_bool(&sentinel),
                5000,
            ).await;

            // Check for rejection
            let rejected = self.check_bool("globalThis.__obscura_await_rejected");
            if rejected {
                let err = self.eval_json(&format!(
                    "String(globalThis.__obscura_objects['{}'] && (globalThis.__obscura_objects['{}'].message || globalThis.__obscura_objects['{}']))",
                    oid, oid, oid
                ))?;
                return Err(format!("Promise rejected: {}", err.as_str().unwrap_or("")));
            }

            self.eval_json("globalThis.__obscura_await_meta")?
        } else {
            self.eval_json(&meta_code)?
        };

        self.object_store.insert(oid.clone(), format!("globalThis.__obscura_objects['{}']", oid));

        if await_promise && return_by_value {
            let json_val = self.eval_json(&format!("globalThis.__obscura_objects['{}']", oid))?;
            return Ok(Self::info_from_json(&json_val));
        }

        Ok(Self::info_from_meta(&meta_result, Some(oid)))
    }

    pub async fn call_function_on_for_cdp(
        &mut self,
        function_declaration: &str,
        object_id: Option<&str>,
        arguments: &[serde_json::Value],
        return_by_value: bool,
        await_promise: bool,
    ) -> Result<RemoteObjectInfo, String> {
        let this_expr = self.resolve_this(object_id);
        let (setup, args_list) = self.build_args(arguments);

        self.object_counter += 1;
        let oid = self.make_oid(self.object_counter);

        if await_promise {
            let done_counter = self.object_counter;
            let err_meta_fn = Self::meta_extract_js("__result");
            let code = format!(
                "(async function() {{\n\
                    {setup}\n\
                    var __fn = ({fn_decl});\n\
                    var __this = ({this_expr});\n\
                    var __result;\n\
                    try {{\n\
                        __result = await __fn.call(__this, {args});\n\
                        globalThis.__obscura_objects['{oid}'] = __result;\n\
                        globalThis.__obscura_await_meta = {meta_fn};\n\
                    }} catch(e) {{\n\
                        __result = e;\n\
                        globalThis.__obscura_objects['{oid}'] = e;\n\
                        globalThis.__obscura_await_meta = {err_meta_fn};\n\
                    }} finally {{\n\
                        globalThis.__obscura_done_{done_counter} = true;\n\
                    }}\n\
                }})()",
                setup = setup,
                fn_decl = function_declaration,
                this_expr = this_expr,
                args = args_list,
                oid = oid,
                meta_fn = Self::meta_extract_js("__result"),
                err_meta_fn = err_meta_fn,
                done_counter = done_counter,
            );

            self.ctx.with(|ctx| ctx.eval::<(), _>(code.as_str()))
                .map_err(|e| format!("JS error: {}", e))?;

            let sentinel = format!("globalThis.__obscura_done_{} === true", done_counter);
            self.resolve_promises_until(|rt| rt.check_bool(&sentinel), 5000).await;

            if return_by_value {
                let json_val = self.eval_json(&format!("globalThis.__obscura_objects['{}']", oid))?;
                return Ok(Self::info_from_json(&json_val));
            }

            let meta_result = self.eval_json("globalThis.__obscura_await_meta")?;
            self.object_store.insert(oid.clone(), format!("globalThis.__obscura_objects['{}']", oid));
            return Ok(Self::info_from_meta(&meta_result, Some(oid)));
        }

        if return_by_value {
            let code = format!(
                "(function() {{\n\
                    {setup}\n\
                    var __fn = ({fn_decl});\n\
                    var __this = ({this_expr});\n\
                    return __fn.call(__this, {args});\n\
                }})()",
                setup = setup,
                fn_decl = function_declaration,
                this_expr = this_expr,
                args = args_list,
            );
            let json_val = self.eval_json(&code)?;
            return Ok(Self::info_from_json(&json_val));
        }

        let code = format!(
            "(function() {{\n\
                {setup}\n\
                var __fn = ({fn_decl});\n\
                var __this = ({this_expr});\n\
                var __result = __fn.call(__this, {args});\n\
                globalThis.__obscura_objects['{oid}'] = __result;\n\
                return {meta_fn};\n\
            }})()",
            setup = setup,
            fn_decl = function_declaration,
            this_expr = this_expr,
            args = args_list,
            oid = oid,
            meta_fn = Self::meta_extract_js("__result"),
        );
        let meta_result = self.eval_json(&code)?;
        self.object_store.insert(oid.clone(), format!("globalThis.__obscura_objects['{}']", oid));
        Ok(Self::info_from_meta(&meta_result, Some(oid)))
    }

    pub async fn call_function_on(
        &mut self,
        function_declaration: &str,
        object_id: Option<&str>,
        arguments: &[serde_json::Value],
        return_by_value: bool,
    ) -> Result<RemoteObjectInfo, String> {
        self.call_function_on_for_cdp(function_declaration, object_id, arguments, return_by_value, false).await
    }

    pub fn store_object(&mut self, js_expression: &str) -> Result<String, String> {
        self.object_counter += 1;
        let oid = self.make_oid(self.object_counter);
        let code = format!("globalThis.__obscura_objects['{}'] = ({});", oid, js_expression);
        self.execute_script("<store>", &code)?;
        self.object_store.insert(oid.clone(), format!("globalThis.__obscura_objects['{}']", oid));
        Ok(oid)
    }

    pub fn store_object_with_meta(
        &mut self,
        js_expression: &str,
    ) -> Result<RemoteObjectInfo, String> {
        self.object_counter += 1;
        let oid = self.make_oid(self.object_counter);
        let code = format!(
            "(function() {{\n\
                var __result = (\n{expr}\n);\n\
                globalThis.__obscura_objects['{oid}'] = __result;\n\
                return {meta_fn};\n\
            }})()",
            expr = js_expression,
            oid = oid,
            meta_fn = Self::meta_extract_js("__result"),
        );
        let meta_result = self.eval_json(&code)?;
        self.object_store.insert(oid.clone(), format!("globalThis.__obscura_objects['{}']", oid));
        Ok(Self::info_from_meta(&meta_result, Some(oid)))
    }

    pub fn release_object(&mut self, object_id: &str) {
        if let Some(expr) = self.object_store.get(object_id) {
            let _ = self.execute_script("<release>", &format!("delete {}", expr));
        }
        self.object_store.remove(object_id);
    }

    pub fn release_object_group(&mut self) {
        self.object_store.clear();
        let _ = self.execute_script("<release-group>", "globalThis.__obscura_objects = {};");
    }

    // -----------------------------------------------------------------------
    // Module loading
    // -----------------------------------------------------------------------

    pub async fn load_module(&mut self, url: &str, budget_ms: u64) -> Result<(), String> {
        let _ = (url, budget_ms);
        // TODO: implement ES module loading via rquickjs Module API
        tracing::warn!("load_module not yet implemented in rquickjs port");
        Ok(())
    }

    pub async fn load_inline_module(&mut self, code: &str, base_url: &str, budget_ms: u64) -> Result<(), String> {
        let _ = (code, base_url, budget_ms);
        tracing::warn!("load_inline_module not yet implemented in rquickjs port");
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Private helpers (ported from original, unchanged JS logic)
    // -----------------------------------------------------------------------

    fn check_bool(&self, code: &str) -> bool {
        self.ctx.with(|ctx| {
            ctx.eval::<bool, _>(code).unwrap_or(false)
        })
    }

    fn make_oid(&self, counter: u64) -> String {
        format!("{{\"injectedScriptId\":1,\"id\":{}}}", counter)
    }

    // Multi-statement code (var/let/const/if/for/while/return) cannot be
    // wrapped in a return expression, so it is executed as a block that
    // returns null on error. Single expressions are wrapped in a return
    // statement so their value is captured for JSON serialization.
    fn wrap_expression(expression: &str) -> String {
        let trimmed = expression.trim();
        let is_multi_statement = trimmed.starts_with("var ")
            || trimmed.starts_with("let ")
            || trimmed.starts_with("const ")
            || trimmed.starts_with("if ")
            || trimmed.starts_with("for ")
            || trimmed.starts_with("while ")
            || trimmed.starts_with("return ");

        if is_multi_statement {
            format!(
                "(function() {{ try {{\n{}\n}} catch(e) {{ return null; }} }})()",
                expression
            )
        } else {
            let cleaned = trimmed.trim_end_matches(|c: char| c == ';' || c.is_whitespace());
            format!(
                "(function() {{ try {{ return (\n{}\n); }} catch(e) {{ return null; }} }})()",
                cleaned
            )
        }
    }

    fn meta_extract_js(var_name: &str) -> String {
        format!(
            r#"(function(v) {{
                var t = typeof v;
                var st = null, cn = '', desc = '';
                if (v === null) {{ t = 'object'; st = 'null'; }}
                else if (v === undefined) {{ t = 'undefined'; }}
                else if (Array.isArray(v)) {{
                    st = 'array'; cn = 'Array';
                    desc = 'Array(' + v.length + ')';
                }}
                else if (t === 'object' && typeof v._nid === 'number') {{
                    st = 'node';
                    cn = v.constructor ? v.constructor.name : 'Node';
                    if (v.nodeType === 9) cn = 'HTMLDocument';
                    else if (v.nodeType === 1) cn = 'HTML' + (v.tagName || 'Element').charAt(0) + (v.tagName || 'Element').slice(1).toLowerCase() + 'Element';
                    desc = v.tagName ? v.tagName.toLowerCase() : (v.nodeName || 'node');
                }}
                else if (t === 'function') {{
                    cn = 'Function';
                    desc = v.name ? 'function ' + v.name + '()' : 'function()';
                }}
                else if (t === 'object') {{
                    cn = (v.constructor && v.constructor.name) || 'Object';
                    desc = cn;
                }}
                else {{ desc = String(v); }}
                return JSON.stringify({{type:t,subtype:st,className:cn,description:desc}});
            }})({var_name})"#,
            var_name = var_name,
        )
    }

    fn resolve_this(&self, object_id: Option<&str>) -> String {
        match object_id {
            Some(oid) => {
                if let Some(retrieval) = self.object_store.get(oid) {
                    retrieval.clone()
                } else if oid.starts_with("node-") {
                    let nid = oid.strip_prefix("node-").unwrap_or("0");
                    format!(
                        "(function() {{ \
                            var nid = {}; \
                            var cache = globalThis._cache || new Map(); \
                            if (cache.has(nid)) return cache.get(nid); \
                            return null; \
                        }})()",
                        nid
                    )
                } else {
                    "globalThis".to_string()
                }
            }
            None => "globalThis".to_string(),
        }
    }

    fn build_args(&self, arguments: &[serde_json::Value]) -> (String, String) {
        let mut setup_lines = Vec::new();
        let mut arg_names = Vec::new();

        for (i, arg) in arguments.iter().enumerate() {
            let arg_name = format!("__arg{}", i);
            if let Some(value) = arg.get("value") {
                let json_str = serde_json::to_string(value).unwrap_or_else(|_| "undefined".to_string());
                setup_lines.push(format!("var {} = {};", arg_name, json_str));
            } else if let Some(oid) = arg.get("objectId").and_then(|v| v.as_str()) {
                if let Some(retrieval) = self.object_store.get(oid) {
                    setup_lines.push(format!("var {} = {};", arg_name, retrieval));
                } else {
                    setup_lines.push(format!("var {} = undefined;", arg_name));
                }
            } else if let Some(unser) = arg.get("unserializableValue").and_then(|v| v.as_str()) {
                setup_lines.push(format!("var {} = {};", arg_name, unser));
            } else {
                setup_lines.push(format!("var {} = undefined;", arg_name));
            }
            arg_names.push(arg_name);
        }

        (setup_lines.join("\n"), arg_names.join(", "))
    }

    fn info_from_json(value: &serde_json::Value) -> RemoteObjectInfo {
        match value {
            serde_json::Value::Null => RemoteObjectInfo {
                js_type: "object".into(),
                subtype: Some("null".into()),
                class_name: String::new(),
                description: "null".into(),
                object_id: None,
                value: Some(serde_json::Value::Null),
            },
            serde_json::Value::Bool(b) => RemoteObjectInfo {
                js_type: "boolean".into(),
                subtype: None,
                class_name: String::new(),
                description: b.to_string(),
                object_id: None,
                value: Some(value.clone()),
            },
            serde_json::Value::Number(n) => RemoteObjectInfo {
                js_type: "number".into(),
                subtype: None,
                class_name: String::new(),
                description: n.to_string(),
                object_id: None,
                value: Some(value.clone()),
            },
            serde_json::Value::String(s) => RemoteObjectInfo {
                js_type: "string".into(),
                subtype: None,
                class_name: String::new(),
                description: s.clone(),
                object_id: None,
                value: Some(value.clone()),
            },
            _ => RemoteObjectInfo {
                js_type: "object".into(),
                subtype: None,
                class_name: "Object".into(),
                description: value.to_string(),
                object_id: None,
                value: Some(value.clone()),
            },
        }
    }

    fn info_from_meta(meta: &serde_json::Value, object_id: Option<String>) -> RemoteObjectInfo {
        let js_type = meta.get("type").and_then(|v| v.as_str()).unwrap_or("undefined").to_string();
        let subtype = meta.get("subtype").and_then(|v| v.as_str()).map(|s| s.to_string());
        let class_name = meta.get("className").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let description = meta.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let value = if js_type == "undefined" || (js_type == "object" && subtype.as_deref() == Some("null")) {
            Some(serde_json::Value::Null)
        } else {
            None
        };
        RemoteObjectInfo { js_type, subtype, class_name, description, object_id, value }
    }
}
