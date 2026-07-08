# GUIDE.md — Porting Obscura to Android with QuickJS (rquickjs)

> This is the master guide. It is written to be executed by a worker who has **no prior
> context** and **cannot browse the web**. Every command is copy-pasteable. Where a
> decision is needed, the decision is already made for you and explained.
>
> There are TWO environments described:
> - **(A) Termux on-device (aarch64 Android phone).** This is the win-win path: you build
>   AND run in the same place, natively, no emulator. Prefer this.
> - **(B) Linux host cross-compile (x86_64).** Used to produce `.so` files for an APK, or
>   when you don't want to build on the phone. Requires the Android NDK + QEMU to test.
>
> If a step says `[TERMUX]` do it only in environment A. If it says `[CROSS]` do it only
> in environment B. Unlabeled steps apply to both.

---

## 0. Mental model — what we are doing and why

Obscura is a headless browser engine in Rust. Its JS runtime crate, `obscura-js`, is built
on **`deno_core`** (Deno's high-level V8 runtime — NOT raw V8). `deno_core` pulls in
`rusty_v8`, and **V8 has no working Android build** (upstream bug, worker threads segfault
under emulation). That is the blocker.

The fix: replace the **JS engine layer** of `obscura-js` with **QuickJS-NG** via the
**`rquickjs`** crate. QuickJS is a few C files, builds with any C compiler, no Chromium
build system, and — verified in this kit — **runs on aarch64 Android**.

What changes:
- `deno_core::JsRuntime`  → `rquickjs::{Runtime, Context}` (or `AsyncRuntime/AsyncContext`).
- `#[op2]` ops            → `rquickjs` native functions / `#[rquickjs::class]` + `Func`.
- `Deno.core.ops.op_*`    → a JS shim object we install so bootstrap.js keeps working.
- V8 startup snapshot     → runtime evaluation of bootstrap.js at startup (QuickJS has no
                            V8 snapshot; it has bytecode, but we start simple).

What DOESN'T change (the whole point of keeping the JS-facing API identical):
- `obscura-dom`, `obscura-net`, `obscura-browser`, `obscura-cdp`, `obscura-cli` — untouched.
- The `window`/`document`/`console`/`fetch` surface that bootstrap.js exposes to page JS.

---

## 1. Phase 0 — Environment setup

### 1A. [TERMUX] On-device setup (primary path)

Run these in Termux on the Android device:

```bash
# Update base system
pkg update -y && pkg upgrade -y

# Toolchain: Rust, clang (provides libclang for bindgen), git, make, cmake
pkg install -y rust clang git make cmake binutils

# Sanity check
rustc --version         # expect 1.7x+ (Termux tracks stable)
cargo --version
clang --version
echo "libclang:"; ls $PREFIX/lib/libclang.so* 2>/dev/null || find $PREFIX -name 'libclang*.so*'
```

Notes for Termux:
- Termux is **already aarch64-linux-android**, so there is **no cross-compilation and no
  `rustup target add`** — `cargo build` produces a native binary you can run immediately.
- Termux's `rquickjs` build uses the `bindgen` feature; `libclang` ships with the `clang`
  package, so bindgen works out of the box.
- If `pkg install rust` gives an old Rust, install via `rustup` inside Termux instead:
  `pkg install rustup && rustup default stable`.

### 1B. [CROSS] Linux x86_64 host setup (for building APK .so files)

This is exactly what was done to verify this kit. Reproduce with `scripts/setup-cross.sh`,
or manually:

```bash
# 1) Rust + Android target
curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable
source "$HOME/.cargo/env"
rustup target add aarch64-linux-android
# optional extra ABIs:
rustup target add armv7-linux-androideabi x86_64-linux-android

# 2) libclang for bindgen (system package)
sudo dnf install -y clang clang-libs llvm-libs     # Amazon Linux / Fedora
# (Debian/Ubuntu: sudo apt-get install -y clang libclang-dev llvm)

# 3) Android NDK r27c
cd $HOME
curl -sSL -o ndk.zip https://dl.google.com/android/repository/android-ndk-r27c-linux.zip
unzip -q ndk.zip           # -> $HOME/android-ndk-r27c

# 4) (test only) a static qemu to run aarch64 binaries on x86_64
curl -sSL -o qemu-aarch64-static \
  https://github.com/multiarch/qemu-user-static/releases/download/v7.2.0-1/qemu-aarch64-static
chmod +x qemu-aarch64-static
```

#### [CROSS] `.cargo/config.toml` (the linker wiring — USE EXACTLY THIS, fix the path)

```toml
# NDK r27c toolchain (linux-x86_64 prebuilt). Target Android API 24.
[target.aarch64-linux-android]
linker = "/home/USER/android-ndk-r27c/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android24-clang"

[target.armv7-linux-androideabi]
linker = "/home/USER/android-ndk-r27c/toolchains/llvm/prebuilt/linux-x86_64/bin/armv7a-linux-androideabi24-clang"

[target.x86_64-linux-android]
linker = "/home/USER/android-ndk-r27c/toolchains/llvm/prebuilt/linux-x86_64/bin/x86_64-linux-android24-clang"
```

#### [CROSS] Environment variables for the build (bindgen + cc must find the NDK)

```bash
NDK=$HOME/android-ndk-r27c
TB=$NDK/toolchains/llvm/prebuilt/linux-x86_64
export LIBCLANG_PATH=/usr/lib64                      # host libclang for bindgen to PARSE with
export CC_aarch64_linux_android="$TB/bin/aarch64-linux-android24-clang"
export AR_aarch64_linux_android="$TB/bin/llvm-ar"
export CFLAGS_aarch64_linux_android="--target=aarch64-linux-android24 --sysroot=$TB/sysroot"
export BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android="--sysroot=$TB/sysroot --target=aarch64-linux-android24"
```

**Why two clangs?** bindgen uses the *host* `libclang` to parse headers, but must be told
(`--sysroot`, `--target`) to read the *Android bionic* headers so the generated FFI matches
the device. `cc` uses the *NDK* clang to actually compile QuickJS's C for the device.

---

## 2. Phase 1 — Spike (PROVE rquickjs runs on Android)  ✅ already verified

The proof is in `spike/`. Do this before touching Obscura.

### 2A. [TERMUX] native build + run (30 seconds, no emulator)

```bash
cd spike
cargo run --release
# EXPECTED OUTPUT:
# [spike] 1 + 1 = 2
# [spike] optional-chaining/?? = 42
# [spike] 2n ** 64n = 18446744073709551616
# [spike] typeof globalThis = object
# [spike] OK
```

If you see that, **rquickjs works on your device. Stop and proceed to Phase 2.**

### 2B. [CROSS] cross-compile + run under QEMU (what this kit did)

```bash
cd spike
# (env vars from 1B must be exported)
cargo build --release --target aarch64-linux-android
file target/aarch64-linux-android/release/qjs-spike
#  -> ELF 64-bit LSB pie executable, ARM aarch64 ... interpreter /system/bin/linker64

# To RUN it on the x86_64 host we need a static build (no bionic linker present):
RUSTFLAGS="-C target-feature=+crt-static" \
  cargo build --release --target aarch64-linux-android
$HOME/qemu-aarch64-static target/aarch64-linux-android/release/qjs-spike
# -> same [spike] ... OK output.  THIS is where V8 would SIGSEGV; QuickJS does not.
```

See `spike/VERIFIED-OUTPUT.txt` for the exact captured run.

---

## 3. Phase 2 — Port obscura-js from deno_core to rquickjs

Full detail lives in `porting/`. This section is the roadmap.

### 3.1 Get the source

```bash
git clone https://github.com/h4ckf0r0day/obscura.git
cd obscura                     # verified at commit 5c3d560
```

The crate to change is **only** `crates/obscura-js`. Its real shape (verified):

| File                | Lines | Role | Port action |
|---------------------|------:|------|-------------|
| `js/bootstrap.js`   | 8001  | Builds window/document/DOM/console/fetch in JS; calls `Deno.core.ops.op_*` | **Keep almost as-is**; provide a `Deno.core.ops` shim |
| `src/runtime.rs`    | 2660  | `JsRuntime` lifecycle, event loop, script exec | Rewrite on `rquickjs` |
| `src/ops.rs`        | 1853  | 22 `#[op2]` ops (the Rust↔JS bridge) | Reimplement as rquickjs functions |
| `src/module_loader.rs`| 115 | `deno_core::ModuleLoader` | Rewrite on `rquickjs` module resolver/loader |
| `src/build.rs`      | 38    | Bakes a **V8 startup snapshot** from bootstrap.js | Remove snapshot; eval bootstrap at startup |
| `src/v8_lock.rs`    | 27    | Serializes V8 isolates across threads | Mostly deletable (QuickJS is 1 runtime/context) |
| `src/cdp_watchdog.rs`| 117  | Kills runaway scripts via V8 isolate handle | Reimplement with `JS_SetInterruptHandler` |
| `src/v8_flags.rs`   | 37    | V8 CLI flags | Delete (no equivalent) |
| `src/markdown.rs`   | 71    | HTML→MD JS blob | Keep (pure JS/data) |

### 3.2 The 22 ops (the actual bridge)  — see `porting/ops-inventory.md`

Breakdown (verified): 16 sync `#[op2]`, 4 `#[op2(fast)]`, 2 `#[op2(async)]`.
Names: `op_dom`, `op_dom_inner`, `op_console_msg`, `op_get_cookies`, `op_set_cookie`,
`op_navigate`, `op_binding_called`, `op_subtle_digest`, `op_subtle_hmac`,
`op_subtle_aes_gcm`, `op_subtle_aes_cbc`, `op_subtle_aes_ctr`, `op_subtle_pbkdf2`,
`op_subtle_hkdf`, `op_random_bytes`, `op_url_parse`, `op_url_set`, `op_url_resolve`,
`op_encoding_for_label`, `op_text_decode`, `op_url_encode_query`, `op_fetch_url`.

### 3.3 The mapping (deno_core → rquickjs) — see `porting/deno_core-to-rquickjs.md`

| deno_core concept | rquickjs equivalent |
|---|---|
| `JsRuntime::new(RuntimeOptions{..})` | `let rt = Runtime::new()?; let ctx = Context::full(&rt)?;` |
| `runtime.execute_script(name, src)` | `ctx.with(\|ctx\| ctx.eval::<T,_>(src))` |
| `#[op2] fn op_x(state,#[string] s)->R` | a Rust `fn` registered via `Func::from` on a JS object |
| `OpState` / `state.borrow::<S>()` | `Ctx::userdata` / captured `Arc<Mutex<S>>` in the closure |
| `#[op2(async)]` (returns Future) | `AsyncContext` + `rquickjs` promise; return `impl Future` |
| `#[serde]` arg/return | `rquickjs_serde` or manual `IntoJs`/`FromJs` |
| `deno_core::ModuleLoader` | `rquickjs::loader::{Loader, Resolver}` |
| V8 startup snapshot | eval bootstrap.js at startup (or QuickJS bytecode later) |
| `run_event_loop()` (pumps promises) | `rt.execute_pending_job()` loop / `AsyncRuntime::idle().await` |

### 3.4 The `Deno.core.ops` shim (KEY trick to avoid rewriting 8001 lines of JS)

bootstrap.js calls ops through a global `Deno.core.ops.op_*`. Instead of editing bootstrap,
install that shape from Rust before evaluating bootstrap:

```
globalThis.Deno = { core: { ops: { /* op_console_msg, op_dom, ... all 22 */ } } };
```

Each property is a Rust function bound with `rquickjs`. bootstrap.js then runs unmodified.
(There are a few non-`ops.` call sites — `Deno.core.<something>` — enumerate them with
`grep -oE 'Deno\.core\.[a-zA-Z.]+' js/bootstrap.js` and shim those too.)

### 3.5 Incremental order (do NOT try to port all at once)

1. Stand up `Runtime`+`Context`, eval `1+1`. (same as spike)
2. Install `console` op (`op_console_msg`) + `Deno.core.ops` skeleton. Eval a tiny script
   that calls `console.log`. Confirm Rust receives it.
3. Eval **bootstrap.js** with all 22 ops stubbed (returning empty/placeholder). Fix each
   error bootstrap throws until it loads clean. This flushes out API-shape mismatches early.
4. Implement the pure ops (no async, no state): url/encoding/random/subtle crypto. These are
   deterministic and easy to unit-test.
5. Implement state ops (`op_dom*`, cookies, navigate, binding) against the shared state.
6. Implement the 2 async ops (`op_fetch_url`, and the other async) on `AsyncContext`; wire
   the promise → Rust future bridge; add the event-loop pump.
7. Reimplement the watchdog via `JS_SetInterruptHandler`.
8. Run obscura-browser/obscura-cdp against it end to end.

### 3.6 Spec gaps to WATCH (flag, don't silently skip)

QuickJS-NG ≈ ES2020 + most later features, but vs V8 on real pages, watch:
- **No JIT** → CPU-heavy page scripts are slower (interpreter). Acceptable for scraping.
- **WeakRef / FinalizationRegistry** — present in NG but verify behavior if bootstrap uses it.
- **Intl** — QuickJS-NG Intl is limited vs V8. If pages depend on `Intl.*`, flag it.
- **structuredClone / specific TypedArray edge cases** — verify against bootstrap usage.
- **RegExp lookbehind / named groups** — supported in NG; still worth a targeted test.
- **Snapshot cold-start**: without V8 snapshot, first-run startup re-parses bootstrap.js
  (~366 KB). Mitigate later with QuickJS bytecode (`JS_WriteObject`) if startup matters.

A skeleton starter crate is in `porting/skeleton/` — copy it over `crates/obscura-js`
incrementally, not wholesale.

---

## 4. Phase 3 — Android app integration (JNI + jniLibs)

Full detail in `android/`. Roadmap:

### 4.1 Build as a cdylib

In `crates/obscura-js/Cargo.toml` (or a thin new `obscura-android` crate that depends on the
workspace), set:

```toml
[lib]
crate-type = ["cdylib"]     # produces .so
```

### 4.2 [CROSS] Produce the .so per ABI and place in jniLibs

```bash
cargo build --release --target aarch64-linux-android      # arm64-v8a
cargo build --release --target armv7-linux-androideabi    # armeabi-v7a
cargo build --release --target x86_64-linux-android       # x86_64 (emulator)

# layout the Android app expects:
mkdir -p app/src/main/jniLibs/{arm64-v8a,armeabi-v7a,x86_64}
cp target/aarch64-linux-android/release/libobscura_android.so app/src/main/jniLibs/arm64-v8a/
cp target/armv7-linux-androideabi/release/libobscura_android.so app/src/main/jniLibs/armeabi-v7a/
cp target/x86_64-linux-android/release/libobscura_android.so app/src/main/jniLibs/x86_64/
```

### 4.3 JNI boundary (sketch in android/jni-boundary.rs)

Use the `jni` crate. Expose e.g. `Java_com_obscura_Engine_startServer` that spawns the CDP
server on a background thread and returns the ws:// port. **Threading assumption** (stated,
not invented): the Android app runs Obscura inside a **foreground service** that owns a
Tokio runtime for the lifetime of the service; JS execution stays on one thread (QuickJS is
single-threaded per runtime), which actually *simplifies* the old V8 `v8_lock` situation.

### 4.4 [TERMUX] alternative: skip JNI entirely for testing

Because Termux is Android, you can run the **CLI binary** directly on the phone and point
Puppeteer/Playwright (or curl) at the CDP ws:// port — no APK, no JNI. This is the fastest
on-device test loop. See `docs/GUIDE.md` §4.4 below.

---

## 5. Fallback: boa_engine (only if rquickjs hits a hard wall)

`boa_engine` is 100% pure Rust (no C toolchain, trivial Android build) but is younger and
slower and ~94% Test262. Keep as documented fallback. Do NOT switch unless a concrete
rquickjs blocker is proven and written down in STATUS.md. rquickjs is verified working here.

---

## 6. Definition of done

- [ ] Phase 1 spike runs on your target (Termux native or QEMU). ✅ done in this kit.
- [ ] bootstrap.js loads under rquickjs with all 22 ops present (stub or real).
- [ ] All 22 ops implemented; unit tests for pure ops pass.
- [ ] `obscura serve` starts, CDP ws:// reachable, a real page fetch+DOM query works.
- [ ] cdylib `.so` built for arm64-v8a (+ other ABIs) and placed in jniLibs.
- [ ] On-device run against a simple test page returns expected DOM/markdown.
