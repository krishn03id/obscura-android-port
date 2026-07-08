# deno_core → rquickjs mapping (the framework swap)

This is the reference for translating each deno_core construct. Pair with the rquickjs 0.12.1
docs (docs.rs/rquickjs/0.12.1). Where the worker cannot browse, the essential API shapes are
inlined below.

## Runtime & context

```rust
// deno_core
let mut js = JsRuntime::new(RuntimeOptions { extensions, startup_snapshot: Some(SNAPSHOT), .. });
js.execute_script("<name>", code)?;
js.run_event_loop(PollEventLoopOptions::default()).await?;

// rquickjs (sync)
use rquickjs::{Runtime, Context};
let rt = Runtime::new()?;
rt.set_max_stack_size(512 * 1024);            // tune; QuickJS default stack is small
let ctx = Context::full(&rt)?;
ctx.with(|ctx| -> rquickjs::Result<()> {
    ctx.eval::<(), _>(code)?;                  // execute_script
    Ok(())
})?;
while rt.is_job_pending() { rt.execute_pending_job()?; }  // ~ run_event_loop for sync jobs

// rquickjs (async — needed for op_fetch_url)
use rquickjs::{AsyncRuntime, AsyncContext};
let rt = AsyncRuntime::new()?;
let ctx = AsyncContext::full(&rt).await?;
async_with!(ctx => |ctx| { ctx.eval::<(),_>(code)?; Ok::<_,rquickjs::Error>(()) }).await?;
rt.idle().await;                              // drain pending jobs/promises
```

## Ops → bound functions

```rust
// deno_core op
#[op2(fast)]
fn op_console_msg(state: &OpState, #[string] level: &str, #[string] msg: &str) { .. }

// rquickjs equivalent: a normal fn, bound onto the Deno.core.ops shim object.
use rquickjs::{Ctx, Function, Object, function::Func};

fn console_msg(level: String, msg: String) {
    match level.as_str() {
        "warn" => tracing::warn!(target:"obscura::console","{msg}"),
        "error"=> tracing::error!(target:"obscura::console","{msg}"),
        _      => tracing::info!(target:"obscura::console","{msg}"),
    }
}

// installation (see bridge.rs skeleton): ops.set("op_console_msg", Func::from(console_msg))?;
```

Return/throw semantics: return `rquickjs::Result<T>` from an op; `Err` becomes a thrown JS
exception automatically (mirrors deno op error propagation). This preserves Obscura's
"panics must not unwind into the engine" rule — see FINDINGS.md §6.

## Shared state (OpState)

```rust
// deno: state.borrow::<SharedState>() / borrow_mut()
// rquickjs option A: userdata
ctx.store_userdata(app_state.clone())?;                 // Arc<Mutex<ObscuraState>>
let st = ctx.userdata::<Arc<Mutex<ObscuraState>>>().unwrap();
// option B (simpler for closures): capture Arc<Mutex<..>> when building each Func
let st = app_state.clone();
ops.set("op_navigate", Func::from(move |url: String| { st.lock().unwrap().navigate(url); }))?;
```

## Serde args/returns

deno `#[serde]` ↔ rquickjs `rquickjs_serde` (feature) or manual `IntoJs`/`FromJs`.
For structs already `Serialize`/`Deserialize`, enable rquickjs serde and use
`rquickjs::serde::{from_value, to_value}` inside the op.

## Byte buffers

deno `#[buffer] &[u8]` / `#[arraybuffer]` ↔ `rquickjs::TypedArray<u8>` (input) and build
output with `TypedArray::new(ctx, &bytes)` or `ArrayBuffer::new(ctx, bytes)`.

## Module loader

```rust
// deno: impl ModuleLoader { resolve(..)->ModuleSpecifier; load(..)->ModuleLoadResponse }
// rquickjs: implement Resolver + Loader
use rquickjs::loader::{Loader, Resolver};
struct ObscuraResolver; impl Resolver for ObscuraResolver { fn resolve(&mut self, ctx:&Ctx, base:&str, name:&str)->rquickjs::Result<String>{..} }
struct ObscuraLoader;   impl Loader   for ObscuraLoader   { fn load(&mut self, ctx:&Ctx, path:&str)->rquickjs::Result<Module>{..} }
rt.set_loader(ObscuraResolver, ObscuraLoader);
```
Note: obscura's bootstrap runs as a classic script, not an ES module, so the module loader is
only needed if pages/`import()` are used. Port it AFTER bootstrap works.

## Watchdog (runaway-script kill)

```rust
// deno: IsolateHandle::terminate_execution()
// rquickjs: interrupt handler polled by the engine
rt.set_interrupt_handler(Some(Box::new(move || {
    should_stop.load(Ordering::Relaxed)   // return true to abort current execution
})));
```
Set a flag from a timer thread (mirror cdp_watchdog.rs timeout logic).

## Snapshot

deno bakes a V8 startup snapshot in build.rs. QuickJS-NG has none. Replace with:
`ctx.with(|ctx| ctx.eval::<(),_>(BOOTSTRAP_JS))?;` at runtime init. Later optimization:
compile bootstrap to QuickJS bytecode (`Module::write_object` / `qjsc`) and embed it to cut
cold-start parse cost. Start WITHOUT this.
