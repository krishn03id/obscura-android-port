# ops-inventory.md — all 22 ops and how to port each

Source: `crates/obscura-js/src/ops.rs` (1853 lines) at commit 5c3d560.
Variants (verified): 16 `#[op2]` (sync), 4 `#[op2(fast)]` (sync, no-alloc fast path),
2 `#[op2(async)]` (return a Future). In rquickjs, "fast" is irrelevant — they all become
ordinary bound functions; only the async ones need the promise/future bridge.

Legend for "rquickjs approach":
- PURE  = deterministic, no shared state → plain Rust fn bound with `Func::from`.
- STATE = touches `SharedState` → closure capturing `Arc<Mutex<ObscuraState>>` (or `Ctx` userdata).
- ASYNC = returns a JS Promise → `AsyncContext` + `rquickjs` future→promise.

| # | op | deno variant | args (deno attrs) | category | rquickjs approach |
|--:|----|----|----|----|----|
| 1 | op_dom | op2 | serde in/out | STATE | bind fn; args via rquickjs-serde; borrow state |
| 2 | op_dom_inner | op2 | serde | STATE | same as op_dom |
| 3 | op_console_msg | op2(fast) | #[string] level, #[string] msg | STATE(logger) | bind fn(String,String)->() |
| 4 | op_get_cookies | op2 | #[string]->#[string] | STATE | bind fn |
| 5 | op_set_cookie | op2 | #[string]... | STATE | bind fn |
| 6 | op_navigate | op2 | #[string] url | STATE | bind fn |
| 7 | op_binding_called | op2(fast) | #[string] name, #[string] payload | STATE | push to pending_binding_calls |
| 8 | op_subtle_digest | op2 | #[buffer]/#[serde] | PURE | crypto; Vec<u8> in/out via TypedArray |
| 9 | op_subtle_hmac | op2 | buffers | PURE | crypto |
|10 | op_subtle_aes_gcm | op2 | buffers | PURE | crypto |
|11 | op_subtle_aes_cbc | op2 | buffers | PURE | crypto |
|12 | op_subtle_aes_ctr | op2 | buffers | PURE | crypto |
|13 | op_subtle_pbkdf2 | op2 | buffers | PURE | crypto |
|14 | op_subtle_hkdf | op2 | buffers | PURE | crypto |
|15 | op_random_bytes | op2(fast) | #[buffer] out | PURE | fill a Uint8Array |
|16 | op_url_parse | op2 | #[string]->#[serde] | PURE | url crate |
|17 | op_url_set | op2 | #[string]... | PURE | url crate |
|18 | op_url_resolve | op2 | #[string],#[string] | PURE | url crate |
|19 | op_encoding_for_label | op2 | #[string]->#[string] | PURE | encoding_rs |
|20 | op_text_decode | op2 | #[buffer]->#[string] | PURE | encoding_rs |
|21 | op_url_encode_query | op2 | #[serde]->#[string] | PURE | url crate |
|22 | op_fetch_url | op2(async) | #[string] url -> future | ASYNC | AsyncContext; reqwest/wreq; resolve promise |

> Exact arg attributes must be re-read from ops.rs per-fn before porting (this table is the
> map, not a substitute for reading the 5-15 line body of each). Command to dump a body:
> `sed -n '/fn op_url_parse/,/^}/p' crates/obscura-js/src/ops.rs`

## Byte-buffer ops (crypto/random/text_decode)

deno's `#[buffer]`/`#[arraybuffer]` give zero-copy `&[u8]`/`&mut [u8]`. In rquickjs use
`rquickjs::TypedArray<u8>` for input and construct a `TypedArray`/`ArrayBuffer` for output.
There is a copy at the boundary (QuickJS doesn't expose deno's fast zero-copy path) — fine
for scraping workloads; note it as a minor perf delta.

## The 2 async ops (the only hard ones)

`op_fetch_url` returns a Future that resolves to the response. In rquickjs:
1. Use `AsyncRuntime` + `AsyncContext`.
2. Bind the op as an `async` Rust fn; rquickjs turns the returned future into a JS Promise.
3. Pump: `while rt.is_job_pending() { rt.execute_pending_job()? }` for sync jobs; for async,
   drive with `ctx.async_with(...).await` and `AsyncRuntime::idle().await`.
4. bootstrap.js re-binds `Deno.core.ops.op_fetch_url` for request interception (see
   runtime.rs ~line 2127). Keep the property writable so that JS reassignment still works.

## State object

deno: `state.borrow::<SharedState>()` / `borrow_mut()`, fields include
`pending_binding_calls: Vec<(String,String)>`. In rquickjs, store the same struct as
`Ctx::store_userdata(Arc<Mutex<ObscuraState>>)` (or capture an `Arc<Mutex<..>>` in each op
closure). Keep the struct definition identical so the rest of obscura-js/browser is unchanged.
