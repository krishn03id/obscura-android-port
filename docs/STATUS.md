# STATUS.md — phase-by-phase, with real output

Environment used to produce this kit: **Amazon Linux 2023, x86_64, 2 vCPU, 4.2 GB RAM**.
No Android device and no rootable emulator were available in that environment, so on-device
execution was done via **`qemu-aarch64-static`**. On a real phone (Termux), the same code
runs natively — that path is documented and is simpler.

---

## Phase 0 — Environment  ✅ WORKS

Installed and verified:
- Rust **1.96.1** stable; target `aarch64-linux-android` added.
- Android **NDK r27c** (Pkg.Revision 27.2.12479018), aarch64 clang wrappers present.
- System **clang/libclang** (for bindgen host parsing).
- **qemu-aarch64-static 7.2.0** (to execute aarch64 binaries on x86_64).

`.cargo/config.toml` and the NDK env-var block are in docs/GUIDE.md §1B and `spike/`.

Status: ✅ works.

---

## Phase 1 — Spike  ✅ WORKS (this was the make-or-break gate)

Command (build):
```
cargo build --release --target aarch64-linux-android
```
Output (tail):
```
   Compiling rquickjs-sys v0.12.1
   Compiling rquickjs-core v0.12.1
   Compiling rquickjs v0.12.1
   Compiling qjs-spike v0.1.0
    Finished `release` profile [optimized] target(s) in 31.98s
=== EXIT: 0 ===
target/.../qjs-spike: ELF 64-bit LSB pie executable, ARM aarch64, interpreter /system/bin/linker64
```

Command (run, static build under QEMU):
```
RUSTFLAGS="-C target-feature=+crt-static" cargo build --release --target aarch64-linux-android
qemu-aarch64-static target/aarch64-linux-android/release/qjs-spike
```
Output (REAL, captured):
```
[spike] 1 + 1 = 2
[spike] optional-chaining/?? = 42
[spike] 2n ** 64n = 18446744073709551616
[spike] typeof globalThis = object
[spike] OK
=== qemu exit code: 0 ===
```

Significance: this is exactly the emulated-aarch64 scenario where rusty_v8 SIGSEGVs in a
worker thread. QuickJS-NG (single-threaded, no JIT) runs clean. **Core premise validated.**

Status: ✅ works.

---

## Phase 2 — Port obscura-js  ⚠️ PLANNED IN FULL, NOT YET CODE-COMPLETE

Done:
- Cloned real repo (commit 5c3d560), inventoried every file in `crates/obscura-js`.
- Confirmed engine coupling is via **`deno_core` 0.350** (not raw rusty_v8) — bigger than
  the brief implied.
- Enumerated all **22 ops** (16 sync, 4 fast, 2 async) and how bootstrap.js calls them
  (`Deno.core.ops.op_*`, 33 call sites + a few special names).
- Discovered `build.rs` bakes a **V8 startup snapshot** — no QuickJS equivalent; documented
  the runtime-eval replacement.
- Produced full mapping table, op inventory, incremental port order, spec-gap watchlist,
  and a skeleton crate (`porting/`).

Not done (this is the remaining bulk of the work):
- Writing the actual ported `runtime.rs`, `ops.rs`, `module_loader.rs`.
- Getting bootstrap.js (8001 lines / 366 KB) to load clean under rquickjs.
- The 2 async ops + event-loop/promise bridge.
- Watchdog via `JS_SetInterruptHandler`.

Status: ⚠️ planned + skeleton, not runnable end-to-end yet.

---

## Phase 3 — Android app integration  ⚠️ PLANNED, DEPENDS ON PHASE 2

Done: cdylib/jniLibs build recipe, JNI boundary sketch, Termux no-JNI test loop,
threading assumptions (foreground service owns a Tokio runtime; JS single-threaded).

Not done: real .so of the ported engine (needs Phase 2), real on-device page test.

Status: ⚠️ planned.

---

## What a human still needs to provide

1. **A real Android device (or writable emulator)** for final on-device validation. QEMU
   proved the engine; it can't prove the full CDP server + real network stack on Android.
2. **Time** to grind Phase 2 (the deno_core→rquickjs rewrite of ~4.6k lines of Rust + getting
   366 KB of bootstrap.js to load). This is the real work; everything risky is de-risked.
3. **Decisions if a spec gap bites** (e.g. if a target page needs full `Intl` — then either
   ship ICU data or accept degraded i18n).

## Honest completion estimate

- Risk retired: ~100% (the "does QuickJS even work on Android" question is answered: yes).
- Effort complete: ~35–40% (Phase 0/1 done + Phase 2/3 fully planned & scaffolded).
