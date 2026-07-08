# FINDINGS.md — corrections to the brief + real source analysis

The original brief warned "something may be outdated." It was. These were verified against
GitHub and crates.io on 2026-07-07.

## 1. The brief's facts were stale

| Brief claimed | Verified reality |
|---|---|
| Obscura ~1 star, 4 commits, v0.1.0, 6 crates | **~17.4k stars, v0.1.9 line (commit 5c3d560), 8 crates** |
| obscura-js "embeds V8 via the v8/rusty_v8 crate" | It embeds **`deno_core` 0.350** (`JsRuntime`, `op2`, module loader, snapshots). rusty_v8 is only a transitive dep. |
| "a large bootstrap.js" | bootstrap.js is **8001 lines / ~366 KB**; `runtime.rs` 2660 lines; `ops.rs` 1853 lines. |
| rquickjs "v137-era / current" framing | rquickjs **0.12.1**; since 0.12 the engine is **QuickJS-NG**; async future↔promise integration present. |

**Why this matters:** the port is NOT "swap rusty_v8 bindings for rquickjs bindings." It is
"replace the `deno_core` runtime framework." That is a larger, but still very tractable, job —
and critically it does NOT touch the V8 build system at all, which is the whole win.

## 2. deno_core coupling surface (what actually must be replaced)

From `grep` across `crates/obscura-js/src`:

- `use deno_core::{JsRuntime, RuntimeOptions};`
- `use deno_core::op2;`  → **22 ops**: 16 `#[op2]`, 4 `#[op2(fast)]`, 2 `#[op2(async)]`.
- `use deno_core::OpState;` → shared state via `state.borrow::<SharedState>()`.
- Module loader: `ModuleLoader`, `ModuleSource`, `ModuleSourceCode`, `ModuleType::JavaScript`,
  `ModuleSpecifier`, `resolve_import`, `ResolutionKind`, `RequestedModuleType`.
- Event loop: `PollEventLoopOptions::default` (5 call sites).
- `deno_core::v8::IsolateHandle` — used by the watchdog to interrupt runaway scripts.
- `build.rs`: `deno_core::snapshot::create_snapshot(...)` bakes bootstrap.js into a **V8
  startup snapshot** at compile time.

## 3. V8-specific files that become obsolete or need reimplementation

- `v8_flags.rs` (37 lines): V8 CLI flags — **delete**, no QuickJS equivalent.
- `v8_lock.rs` (27 lines): a process-wide mutex serializing V8 isolates across a
  `tokio::LocalSet` because "only one Isolate may be entered per OS thread." QuickJS has one
  `Runtime`/`Context` and is single-threaded per runtime — **most of this disappears**; the
  concurrency model actually gets simpler (one runtime per Page, no shared-isolate abort).
- `cdp_watchdog.rs` (117 lines): kills runaway scripts via `IsolateHandle::terminate_execution`.
  QuickJS equivalent: `JS_SetInterruptHandler` (rquickjs exposes an interrupt handler) —
  **reimplement**, semantics map cleanly.
- `build.rs` snapshot: QuickJS-NG has no V8 snapshot. Options: (a) eval bootstrap.js at
  startup (simple, slightly slower cold start), (b) precompile to QuickJS bytecode via
  `JS_WriteObject`/`qjsc` and embed the bytecode (fast cold start, more work). Start with (a).

## 4. The bootstrap.js bridge (why the port can be surgical)

bootstrap.js reaches Rust through a single shape: `Deno.core.ops.op_*` (33 call sites) plus a
few `Deno.core.<x>` helpers and dynamic re-binding of `op_fetch_url` for interception. If we
install a `globalThis.Deno.core.ops` object of Rust-backed functions BEFORE evaluating
bootstrap.js, the 8001 lines of JS can run **essentially unmodified**. That keeps the
`window`/`document`/`console`/`fetch` API identical, so `obscura-dom`/`obscura-browser` don't
change. This is the single most important lever for keeping the port small.

## 5. rusty_v8 Android status (unchanged, still blocked)

Confirmed the upstream situation the brief described is still the state of the world: Android
support in rusty_v8 was briefly present, broke, and has no landed fix (worker-thread SIGSEGV
under emulation). This is not a Rust-side fixable issue. Hence the engine swap.

## 6. One more constraint carried over from Obscura's Cargo config

Obscura pins panic strategy carefully because ops must not unwind across the FFI boundary
into the engine. Preserve that discipline in the rquickjs port: wrap op bodies so a Rust
panic becomes a thrown JS error (`ctx.throw(...)`) rather than an unwind. rquickjs already
converts `Result::Err` into a thrown JS exception, so return `rquickjs::Result` from ops and
use `catch_unwind` only where a panic is genuinely possible.
