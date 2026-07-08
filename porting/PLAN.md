# PLAN.md — Phase 2 execution plan (obscura-js: deno_core → rquickjs)

Work top-to-bottom. Each step has an EXIT TEST — do not advance until it passes.
All paths are relative to the cloned `obscura/` repo unless noted.

## Step 0 — Branch & baseline
- `git checkout -b android-quickjs`
- Confirm current tree builds on your host for the normal target FIRST (so you know your
  baseline). If V8 won't build for you at all, skip — you're replacing it anyway.
- EXIT TEST: repo cloned at commit 5c3d560, branch created.

## Step 1 — Swap the crate's deps & scaffolding
- Replace `crates/obscura-js/Cargo.toml` with `skeleton/Cargo.toml` (merge kept deps from the
  original — re-read the original for the exact crypto/html5ever versions).
- Copy `skeleton/src/{lib,runtime,bridge,state}.rs` into `crates/obscura-js/src/`, keeping the
  original `markdown.rs`. Delete `v8_flags.rs`, `v8_lock.rs`. Keep `cdp_watchdog.rs` for now
  but stop importing it.
- Remove `build.rs` snapshot logic (bootstrap is now `include_str!` at runtime).
- EXIT TEST: `cargo build -p obscura-js` fails ONLY on missing op impls / state fields, not on
  deno_core. (i.e., deno_core is fully gone.)

## Step 2 — Boot with stub ops
- Fill `state.rs` to match the real `ObscuraState`/`SharedState` (grep the original).
- Keep all 22 ops as stubs in bridge.rs (return placeholder values).
- EXIT TEST: the `boots_and_evals` unit test passes: runtime constructs, bootstrap.js loads
  without throwing, `1+1` evals. Fix each bootstrap load error until clean. THIS flushes out
  most API-shape mismatches. Expect to iterate here.

## Step 3 — Pure ops (deterministic, unit-testable)
Implement, with a unit test each:
- url: op_url_parse, op_url_set, op_url_resolve, op_url_encode_query
- encoding: op_encoding_for_label, op_text_decode
- random: op_random_bytes
- crypto: op_subtle_{digest,hmac,aes_gcm,aes_cbc,aes_ctr,pbkdf2,hkdf}
- Byte I/O via `rquickjs::TypedArray<u8>` (see ops-inventory.md "byte-buffer ops").
- EXIT TEST: `cargo test -p obscura-js` green for all pure ops; values match the original
  (cross-check a couple against the V8 build or known vectors).

## Step 4 — State ops
- op_console_msg, op_binding_called (done in skeleton), op_dom, op_dom_inner, op_get_cookies,
  op_set_cookie, op_navigate. These read/write `ObscuraState`.
- EXIT TEST: a script exercising console + a DOM query returns expected data; binding calls
  land in `pending_binding_calls`.

## Step 5 — Async op + event loop
- Move runtime to `AsyncRuntime`/`AsyncContext`.
- Implement op_fetch_url (reqwest/wreq), returned as a JS Promise; keep it writable for
  bootstrap's interception reassignment (runtime.rs ~2127 in the original).
- Add the event-loop pump: `rt.idle().await` + `execute_pending_job` for micro-tasks.
- EXIT TEST: `await fetch(url)` from page JS resolves with a real response; a Promise chain
  and `setTimeout(0)`-style microtask ordering behave.

## Step 6 — Watchdog
- Reimplement runaway-script termination via `rt.set_interrupt_handler` driven by a timer
  flag (port cdp_watchdog.rs timeout semantics).
- EXIT TEST: an infinite loop in page JS is aborted after the timeout instead of hanging.

## Step 7 — Wire the workspace
- Ensure obscura-browser/obscura-cdp compile against the new obscura-js unchanged (that's the
  success criterion for "identical JS-facing API").
- EXIT TEST: `cargo build` for the whole workspace (host target) succeeds; `obscura serve`
  starts and a CDP client connects.

## Step 8 — Spec-gap sweep
- Run the repo's JS test suite (there is one — commit 5c3d560 is literally a JS test commit:
  "lock script-error isolation for the babel-polyfill case"). Note any failures tied to
  Intl / JIT-perf / edge features (see GUIDE.md §3.6) and record them in STATUS.md rather
  than silently patching bootstrap.
- EXIT TEST: test suite runs; failures are enumerated and categorized (blocker vs degraded).

## Ordering rationale
Pure→state→async→watchdog is strictly increasing difficulty and dependency. bootstrap-with-
stubs (Step 2) is deliberately early because it surfaces the largest class of bugs (JS↔Rust
shape mismatches) before you've invested in op internals.
