---
name: deno-core-to-rquickjs-port
description: How to port a Rust crate from deno_core (V8) to rquickjs (QuickJS-NG), covering deps, state types, the Deno.core.ops shim, base64 bridge for byte ops, async queue pattern, watchdog sync-runtime limitation, and real-world build/test results.
source: auto-skill
extracted_at: '2026-07-08T00:00:00.000Z'
---

# Porting deno_core → rquickjs (QuickJS-NG)

## When to use
A Rust crate uses `deno_core` (which pulls in `rusty_v8`/V8) and you need to
swap the JS engine to `rquickjs` (QuickJS-NG) — e.g. for Android/aarch64
where V8 segfaults. The crate exposes a JS-facing API (window/document/console/fetch)
via a large bootstrap.js and `#[op2]` ops.

## Core approach: the "ops shim" lever

The single most important technique: **do NOT rewrite the JS**.
bootstrap.js calls Rust through one shape: `Deno.core.ops.op_*`. Install that
shape from Rust *before* evaluating bootstrap.js:

```rust
pub fn install_ops(ctx: &Ctx, state: SharedState) -> rquickjs::Result<()> {
    let global = ctx.globals();
    let deno = Object::new(ctx.clone())?;
    let core = Object::new(ctx.clone())?;
    let ops = Object::new(ctx.clone())?;
    ops.set("op_console_msg", Func::from(|level: String, msg: String| { ... }))?;
    ops.set("op_dom", Func::from(move |cmd: String, a1: String, a2: String| { ... }))?;
    // ... all ops ...
    core.set("ops", ops)?;
    deno.set("core", core)?;
    global.set("Deno", deno)?;
    Ok(())
}
```

Then `ctx.eval::<(), _>(BOOTSTRAP_JS)?` runs the 8000+ lines unmodified.

**Find all call sites first:** `grep -oE 'Deno\.core\.[a-zA-Z._]+' js/bootstrap.js | sort -u`

**CRITICAL ORDERING BUG:** If any JS wrapper code (e.g. crypto wrappers) references
`globalThis.Deno.core.ops`, it will fail silently with "Exception" unless you
`global.set("Deno", deno)?;` FIRST, then install the JS wrappers AFTER. Do not
put the `global.set("Deno", deno)?;` at the end of install_ops if any JS eval
happens before it. Split: install Rust ops → set Deno global → install JS wrappers.

## Step-by-step procedure

### 1. Cargo.toml — swap deps
- Remove `[build-dependencies] deno_core`
- Remove `deno_core = "..."` from `[dependencies]`
- Add `rquickjs = { version = "0.12.1", features = ["full-async", "bindgen", "macro"] }`
  - `bindgen` = generate FFI for targets without prebuilt bindings (needs libclang)
  - `full-async` = AsyncRuntime/AsyncContext for async ops
  - `macro` = `#[rquickjs::class]` / `#[rquickjs::function]` helpers
- Keep all other deps (crypto, url, encoding_rs, etc.) unchanged

### 2. build.rs — remove V8 snapshot
Delete all `deno_core::snapshot::create_snapshot(...)` logic. QuickJS has no
V8 snapshot. Just `include_str!("../js/bootstrap.js")` at runtime.

### 3. SharedState type migration — CRITICAL
deno_core uses `Rc<RefCell<T>>` (single-threaded, no Send). rquickjs async ops
require `Send`. Change:
```rust
// Before (deno_core)
pub type SharedState = Rc<RefCell<ObscuraState>>;
// After (rquickjs)
pub type SharedState = Arc<Mutex<ObscuraState>>;
```
Then mass-replace all `.borrow()` → `.lock().unwrap()` and
`.borrow_mut()` → `.lock().unwrap()` across ops.rs and runtime.rs.

**WARNING:** `std::cell::Ref::map(...)` does not work with `MutexGuard`.
Any method returning `Ref<'_, T>` (e.g. `dom_ref()`) must be removed or
restructured to not return a borrow.

### 4. eval() API — wants &str not &String
rquickjs `ctx.eval()` does NOT accept `&String`. Use `.as_str()`:
```rust
// WRONG: ctx.eval::<(), _>(&code)   // &String — compile error
// RIGHT: ctx.eval::<(), _>(code.as_str())
```
This is easy to miss because the error message is cryptic
(`Vec<u8>: From<&String>` not satisfied).

### 5. Module API — Module::declare, not Module::new
rquickjs 0.12.1 has no `Module::new()`. Use `Module::declare()`:
```rust
let module = Module::declare(ctx.clone(), "<module>", code.as_str())?;
let (module, _promise) = module.eval()?;
```

### 6. Module Loader trait signatures (rquickjs 0.12.1)
The `Resolver` and `Loader` traits take an extra `import_attributes` param:
```rust
impl Resolver for MyResolver {
    fn resolve(&mut self, _ctx: &Ctx, base: &str, name: &str,
        _import_attributes: Option<rquickjs::loader::ImportAttributes>,
    ) -> rquickjs::Result<String> { ... }
}
impl Loader for MyLoader {
    fn load(&mut self, ctx: &Ctx, path: &str,
        _import_attributes: Option<rquickjs::loader::ImportAttributes>,
    ) -> rquickjs::Result<Module> { ... }
}
```

### 7. CRITICAL: Value lifetime issue — use base64 bridge for byte ops

**The problem:** Closures registered via `Func::from` that take `Value` args
and return `Value` (e.g. crypto ops that accept `TypedArray` and return
`TypedArray`) hit **unfixable lifetime errors**:

```
error: lifetime may not live long enough
  --> closure receives `Value<'1>` but returns `Result<Value<'2>, Error>`
```

rquickjs's `Func::from` gives each `Value` parameter its own lifetime, but the
return `Value` needs to outlive all of them. There is no way to satisfy this
with the sync `Context` API.

**The solution — base64 dispatch pattern:** Instead of passing byte arrays as
`Value`, use a **single Rust dispatch function** that takes/returns base64
strings (plain `String`, no lifetime issues), and install **JS wrappers** that
convert `TypedArray` ↔ base64:

```rust
// Rust side: one dispatch fn taking JSON + base64, returning JSON + base64
ops.set("op_crypto_dispatch", Func::from(|cmd: String, args_json: String| -> String {
    crypto_dispatch(&cmd, &args_json)  // returns {"result":"<b64>"} or {"error":"..."}
}))?;
ops.set("op_random_bytes_b64", Func::from(|len: u32| -> String {
    BASE64.encode(&op_random_bytes_impl(len).unwrap())
}))?;

// Then install JS wrappers AFTER global.set("Deno", deno):
ctx.eval::<(), _>(r#"
    (function() {
        function toB64(ta) { /* Uint8Array → btoa */ }
        function fromB64(b64) { /* atob → Uint8Array */ }
        var ops = globalThis.Deno.core.ops;
        var d = ops.op_crypto_dispatch;
        ops.op_subtle_digest = function(alg, data) {
            var r = d("digest", JSON.stringify([alg, toB64(data)]));
            var p = JSON.parse(r);
            if (p.error) throw new Error(p.error);
            return fromB64(p.result);
        };
        // ... same for hmac, aes_gcm, aes_cbc, aes_ctr, pbkdf2, hkdf
        ops.op_text_decode = function(label, data, fatal, ignore_bom) {
            return ops.op_text_decode_b64(label, toB64(data), fatal, ignore_bom);
        };
    })();
"#)?;
```

This pattern eliminates ALL `Value` lifetime errors. The base64 overhead is
negligible for crypto workloads (small buffers).

### 9. Watchdog — interrupt handler replaces IsolateHandle
```rust
rt.set_interrupt_handler(Some(Box::new(move || {
    should_stop.load(Ordering::SeqCst)  // true = abort execution
})));
```
Wrap in an `InterruptHandle { should_stop: Arc<AtomicBool> }` struct that
can be cloned and triggered from a timer thread.

**CRITICAL LIMITATION (verified on rquickjs 0.12.1):** The interrupt handler
fires once at `JS_CallInternal` entry but **does NOT fire during tight
`while(true){}` loops in the sync `Runtime`/`Context` API.** Confirmed with an
AtomicU64 counter — the handler is called once during a quick `1+1` eval but
never during `while(true){}`. The rquickjs test suite only tests the interrupt
handler with `AsyncRuntime`/`AsyncContext`. QuickJS-NG's C source calls
`js_poll_interrupts(ctx)` every 10000 backward jumps (OP_goto), so the issue
is in rquickjs's wrapper, not QuickJS itself.

**Fix:** Switch to `AsyncRuntime`/`AsyncContext` for the watchdog to work.
This is a larger refactor but is required for production safety. Until then,
`evaluate_with_timeout` does NOT kill infinite loops.

### 10. v8_lock.rs — simplify to no-op
QuickJS is single-threaded per runtime. The process-wide V8 serialization
lock becomes a simple `tokio::sync::Mutex<()>` kept for API compatibility
(the CDP dispatcher still uses it to serialize commands).

### 11. v8_flags.rs — delete
No V8 flags in QuickJS. Keep a no-op `pub fn set_v8_flags(_flags: &str) {}`
for API compatibility with callers.

## Known hard problem: async ops + Promise bridge

op_fetch_url (async) and op_sleep need to return a JS Promise. The naive
approach of creating a Promise via JS eval and resolving it from a spawned
tokio task hits lifetime errors:

```
error: lifetime may not live long enough
  --> closure captures `Ctx<'1>` but returns `Value<'2>`
```

The closure receives `Ctx` (borrowed from the context) but the returned
`Value` borrows from it with a different lifetime. **This is the hardest
part of the port and may require:**
- Using `rquickjs::Promise` API directly (if available in the version)
- Using `AsyncContext` + `async_with!` macro instead of sync `Context`
- Storing async results in state and resolving on event-loop pump
  (the approach used in this port: `pending_async_results: HashMap<usize,
  Result<String, String>>` drained by `resolve_completed_async()`)

## Files to create/modify

| File | Action |
|------|--------|
| Cargo.toml | Swap deps (remove deno_core, add rquickjs) |
| build.rs | Remove snapshot; just `rerun-if-changed` |
| src/lib.rs | Remove v8_flags, add state module, keep markdown |
| src/state.rs | NEW: ObscuraState struct (mirror original fields) |
| src/ops.rs | Reimplement all ops as plain fns + install_ops() bridge |
| src/runtime.rs | Rewrite on rquickjs Runtime/Context |
| src/module_loader.rs | Port to rquickjs Loader/Resolver traits |
| src/cdp_watchdog.rs | Reimplement via interrupt handler |
| src/v8_lock.rs | Simplify to tokio::sync::Mutex no-op |
| src/v8_flags.rs | Delete or no-op stub |
| src/markdown.rs | Keep unchanged (pure JS/data) |

## Incremental order (do NOT port all at once)

1. Stand up Runtime+Context, eval `1+1` (same as spike)
2. Install ops shim with stubs, eval bootstrap.js — fix each load error
3. Implement pure ops (url, encoding, random, crypto) — unit testable
4. Implement state ops (dom, cookies, navigate, binding)
5. Implement async ops (fetch) — hardest, do last
6. Reimplement watchdog
7. Wire workspace (obscura-browser/obscura-cdp compile unchanged)

## Adding a new async op (post-port)

Once the port is done, adding another async op (e.g. `op_web_search`)
requires changes in **three places** — missing any one causes silent hangs
or wrong-resolver bugs.

### Step 1 — Rust impl + JS wrapper (ops.rs)

Implement the async Rust function, then install a JS wrapper that uses
the **queue + resolver map pattern** (same shape as `op_fetch_url`):

```rust
// Rust impl
pub async fn op_web_search_impl(query: &str) -> Result<String, String> { ... }

// In install_ops(), AFTER global.set("Deno", deno):
ctx.eval::<(), _>(r#"
    globalThis.__obscura_search_queue = [];
    globalThis.__obscura_search_id = 0;
    globalThis.__obscura_search_resolvers = {};
    globalThis.Deno.core.ops.op_web_search = function(query) {
        var id = ++globalThis.__obscura_search_id;
        return new Promise(function(resolve, reject) {
            globalThis.__obscura_search_resolvers[id] = {resolve: resolve, reject: reject};
            globalThis.__obscura_search_queue.push({ id: id, query: query });
        });
    };
"#)?;
```

### Step 2 — Drain method (runtime.rs)

Add a `drain_<name>_queue()` method that reads the JS queue, spawns a
`tokio::task` per item, and inserts results into `pending_async_results`:

```rust
fn drain_search_queue(&self) {
    let queue_json = /* eval JS to read + clear __obscura_search_queue */;
    for item in items {
        let id = item["id"].as_u64().unwrap() as usize;
        let query = item["query"].as_str().unwrap().to_string();
        let st = self.state.clone();
        tokio::spawn(async move {
            let result = crate::ops::op_web_search_impl(&query).await;
            st.lock().unwrap().pending_async_results.insert(id, result);
        });
    }
}
```

Call it inside `pump_jobs_and_async()` right after the existing drain calls:
```rust
self.drain_fetch_queue();
self.drain_search_queue();  // <-- add
```

### Step 3 — CRITICAL: extend `resolve_completed_async()` (runtime.rs)

**This is the bug that bit us.** All async results (fetch, search, future ops)
land in the same `pending_async_results` map keyed by numeric ID. But
`resolve_completed_async()` only knew about `__obscura_fetch_resolvers`. A
search result would complete and sit in the map forever — the Promise never
resolved, the eval hung silently.

**Fix:** when adding a new async op, extend the resolve code to check BOTH
resolver maps. Since IDs from independent counters can collide, check the
new op's resolver map first, then fall back to fetch:

```rust
fn resolve_completed_async(&self) {
    let completed = { self.state.lock().unwrap().pending_async_results.drain().collect::<Vec<_>>() };
    self.ctx.with(|ctx| {
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
                        }})()"#, id = id, val = json_val);
                    let _ = ctx.eval::<(), _>(code.as_str());
                }
                Err(err) => { /* same pattern, r.reject(new Error(...)) */ }
            }
        }
        Ok(())
    });
}
```

**Better long-term fix:** tag each result with its op type so the resolver
map is selected unambiguously instead of by fallback ordering.

### Testing async ops — the CLI trap

`obscura scrape -e 'await Deno.core.ops.op_foo(...)'` **will silently return
`null`/`{}`** — the CLI's `evaluate_with_timeout` serializes the pending
Promise via `JSON.stringify` (which yields `{}`) and returns immediately
without pumping the event loop for bare evals (no `--selector`/`--dump`).

**Correct test method:** write a standalone example binary in
`crates/<crate>/examples/test_<op>.rs` that manually pumps
`run_event_loop()`:

```rust
use std::time::Duration;
use obscura_js::runtime::ObscuraJsRuntime;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut rt = ObscuraJsRuntime::new();
    // Kick off async op, store result in a global flag
    rt.execute_script("kickoff", r#"
        globalThis.__done = false; globalThis.__result = null;
        Deno.core.ops.op_web_search("query")
            .then(r => { globalThis.__result = r; globalThis.__done = true; })
            .catch(e => { globalThis.__result = "ERR:" + e.message; globalThis.__done = true; });
    "#).unwrap();
    // Pump event loop until done or timeout
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        rt.run_event_loop().await.ok();
        tokio::time::sleep(Duration::from_millis(100)).await;
        if rt.evaluate("globalThis.__done").unwrap_or_default() == serde_json::Value::Bool(true) {
            println!("{}", rt.evaluate("globalThis.__result").unwrap());
            break;
        }
        if std::time::Instant::now() >= deadline { break; }
    }
}
```

Run with: `cargo run --release -p obscura-js --example test_search`

### DDG HTML parsing gotcha

When scraping `https://html.duckduckgo.com/html/?q=...`, result anchors are
`<a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=ENCODED_URL&rut=...">`.
**`href` comes AFTER `class`, not before** — a backward search for `href=`
from the `class="result__a"` position grabs the wrong (previous) href. Search
**forward** from `class="result__a"` to find `href=`. The `uddg=` parameter
must be URL-decoded to get the real target URL.
