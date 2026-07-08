# Obscura Android Port

> **Credits:** This project is built on top of [Obscura](https://github.com/h4ckf0r0day/obscura) by [h4ckf0r0day](https://github.com/h4ckf0r0day). The original Obscura engine, DOM implementation, CDP server, and MCP tooling are retained unchanged — this project replaces only the JavaScript runtime (V8 → QuickJS-NG) and adds Android/Termux support.

Porting the Obscura headless browser engine from **V8** (`deno_core`) to **QuickJS-NG** (`rquickjs 0.12.1`) so it builds and runs natively on Android / aarch64 (Termux).

V8 has no working Android build — worker threads SIGSEGV under emulation. QuickJS-NG is a few C files, builds with any C compiler, and runs clean on aarch64 Android.

---

## Tech Stack

| Component | Version |
|-----------|---------|
| Rust | 1.96.1 stable |
| rquickjs (QuickJS-NG) | 0.12.1 |
| Android NDK | r27c (cross-compile only) |
| Kotlin / Android SDK 35 | webshot app |
| reqwest / tokio | networking & async |
| html5ever | DOM parsing |

---

## Project Structure

```
obscura-android-port/
├── README.md                ← you are here
├── start.md                 ← AI agent prompt (executable task brief)
│
├── docs/                    ← all documentation
│   ├── GUIDE.md             ← master copy-paste guide (START HERE)
│   ├── STATUS.md            ← phase-by-phase progress + real output
│   ├── FINDINGS.md          ← source analysis & corrections to original brief
│   └── screenshot.md        ← webshot app design blueprint
│
├── scripts/                 ← environment setup
│   ├── setup-termux.sh      ← on-device: installs Rust + clang via pkg
│   └── setup-cross.sh       ← x86_64 host: installs Rust + NDK + QEMU
│
├── spike/                   ← Phase 1: rquickjs proof-of-concept (DONE)
│   ├── src/main.rs          ← evals 1+1, BigInt, optional chaining
│   ├── Cargo.toml           ← rquickjs 0.12.1 with bindgen
│   ├── Cargo.lock
│   ├── build-termux.sh      ← native Termux build
│   ├── build-cross.sh       ← cross-compile for aarch64-android
│   ├── VERIFIED-OUTPUT.txt  ← captured build + run output
│   └── qjs-spike.aarch64-android  ← prebuilt binary
│
├── porting/                 ← Phase 2: deno_core → rquickjs port (DONE in ~/obscura)
│   ├── PLAN.md              ← 8-step incremental port plan
│   ├── deno_core-to-rquickjs.md  ← API mapping reference
│   ├── ops-inventory.md     ← all 22 ops: names, args, categories
│   └── skeleton/            ← starter crate (copy into obscura/crates/obscura-js)
│       ├── src/
│       │   ├── lib.rs
│       │   ├── bridge.rs    ← op shims (stubs)
│       │   ├── runtime.rs   ← ObscuraRuntime wrapper
│       │   └── state.rs     ← shared state struct
│       ├── build.rs
│       └── Cargo.toml
│
├── android/                 ← Phase 3: JNI boundary (planned)
│   └── jni-boundary.rs      ← stub: nativeEval / nativeStartCdp
│
└── webshot/                 ← standalone WebView screenshot service (WORKING)
    ├── src/com/webshot/
    │   ├── Capturer.kt      ← off-screen WebView → PNG
    │   ├── HttpServer.kt    ← raw socket HTTP on 127.0.0.1:8899
    │   ├── ShotService.kt   ← foreground service host
    │   └── MainActivity.kt  ← start/stop/test UI
    ├── AndroidManifest.xml
    ├── build.sh             ← Gradle-free APK build (aapt2→kotlinc→d8→sign)
    ├── webshot.apk          ← signed APK (787 KB)
    └── webshot.keystore
```

---

## Setup & Installation

### Option A: Termux (on-device, recommended)

```bash
# 1. Install Termux from F-Droid (not Google Play)
# 2. Run setup
bash scripts/setup-termux.sh

# 3. Verify rquickjs works on your device
cd spike && bash build-termux.sh
# Expected: [spike] 1 + 1 = 2 ... [spike] OK
```

### Option B: Cross-compile (x86_64 Linux host)

```bash
bash scripts/setup-cross.sh
# Sets up: Rust + aarch64 target, NDK r27c, QEMU

# Export NDK env vars (see docs/GUIDE.md §1B), then:
cd spike && bash build-cross.sh
```

---

## Usage

### Spike — verify QuickJS runs on Android

```bash
cd spike
cargo run --release
# [spike] 1 + 1 = 2
# [spike] optional-chaining/?? = 42
# [spike] 2n ** 64n = 18446744073709551616
# [spike] typeof globalThis = object
# [spike] OK
```

### Obscura CLI — after porting (in ~/obscura)

```bash
cd ~/obscura
cargo build --release

# Fetch a page
./target/release/obscura fetch https://example.com

# Start CDP server (Puppeteer/Playwright compatible)
./target/release/obscura serve --port 9222

# Web search via JS eval
./target/release/obscura scrape -e 'Deno.core.ops.op_web_search("what is cron job")' \
  "data:text/html,<html></html>"
```

### WebShot — Android WebView screenshot service

```bash
# Build the APK
cd webshot && bash build.sh

# Install
termux-open webshot/webshot.apk

# Tap "Start Service" in the app, then:
curl -X POST http://127.0.0.1:8899/shot \
  -H "Content-Type: application/json" \
  -d '{"url":"https://news.ycombinator.com","full_page":true,"wait_ms":2000}' \
  -o screenshot.png
```

---

## HTTP API (WebShot)

| Method | Endpoint | Body | Response |
|--------|----------|------|----------|
| `GET` | `/health` | — | `{"status":"ok"}` |
| `POST` | `/shot` | JSON (below) | PNG bytes |

**POST /shot JSON parameters:**

```json
{
  "url": "https://example.com",
  "html": "<optional raw HTML instead of URL>",
  "width": 1920,
  "height": 1080,
  "full_page": true,
  "wait_ms": 2000,
  "user_agent": "optional custom UA"
}
```

---

## Build Scripts

| Script | Purpose |
|--------|---------|
| `scripts/setup-termux.sh` | Install Rust, clang, build tools in Termux |
| `scripts/setup-cross.sh` | Install Rust, NDK r27c, QEMU on x86_64 Linux |
| `spike/build-termux.sh` | Build + run spike natively in Termux |
| `spike/build-cross.sh` | Cross-compile spike for aarch64-android |
| `webshot/build.sh` | Build webshot APK without Gradle (aapt2→kotlinc→d8→zipalign→apksigner) |

---

## Environment Variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `OBSCURA_FETCH_TIMEOUT_MS` | `30000` | HTTP fetch timeout per request |
| `OBSCURA_ALLOW_PRIVATE_NETWORK` | unset | Set to `1` to allow loopback/RFC1918 fetches |
| `OBSCURA_NETWORK_BODY_BUFFER_ENTRIES` | `128` | Max buffered network response bodies |
| `OBSCURA_NETWORK_BODY_BUFFER_BYTES` | `2097152` | Max body size to buffer (2 MB) |

---

## Architecture

### What changed in the port

The `obscura-js` crate (in `~/obscura/crates/obscura-js/`) was rewritten:

```
deno_core (V8)                    →  rquickjs (QuickJS-NG)
─────────────────────────────────────────────────────────
JsRuntime                         →  rquickjs::Runtime + Context
#[op2] / OpState                  →  plain Rust functions + JS wrappers
V8 snapshot (build.rs)            →  include_str! + runtime eval
IsolateHandle (watchdog)          →  JS_SetInterruptHandler
Rc<RefCell<>> state              →  Arc<Mutex<>> (thread-safe)
deno_core PollEventLoop           →  manual pump_jobs_and_async loop
```

**Key patterns:**
- **Base64 bridge** — Crypto/byte ops convert `TypedArray ↔ base64` at the JS↔Rust boundary to avoid rquickjs `Value` lifetime issues
- **Async queue** — `op_fetch_url` and `op_web_search` use a JS-side Promise + queue array; the runtime drains the queue, spawns tokio tasks, and resolves promises via `resolve_completed_async()`
- **Watchdog** — A background thread with a condvar timer calls `set_should_stop(true)` on the `InterruptHandle`; QuickJS checks this via the interrupt handler and aborts

### WebShot architecture

```
MainActivity → ShotService (foreground)
                   ├── Capturer (WebView on main thread → Bitmap → PNG)
                   └── HttpServer (ServerSocket on :8899)
                          GET /health  → JSON
                          POST /shot   → PNG
```

The WebView uses `LAYER_TYPE_SOFTWARE` for off-screen rendering. Full-page capture resizes the WebView to `contentHeight` before drawing to canvas.

---

## Known Limitations

- **No JIT** — QuickJS is an interpreter. JS-heavy pages run 10–100× slower than V8. Fine for scraping, slow for SPAs.
- **No `Intl` full support** — QuickJS-NG has basic `Intl`; pages requiring full ICU data may degrade.
- **Full-page screenshots** — `webView.contentHeight` can return 0 in software layer mode on some devices. Workaround: use `evaluateJavascript("document.body.scrollHeight")`.
- **Single-threaded JS** — QuickJS is single-threaded. The runtime uses `ctx.with()` to serialize all JS access.
- **Module loading** — `load_module` / `load_inline_module` are stubs (not yet implemented in the port).

---

## Documentation

Read in this order:

1. **[docs/GUIDE.md](docs/GUIDE.md)** — master guide, copy-paste commands for all phases
2. **[docs/STATUS.md](docs/STATUS.md)** — phase-by-phase status with real captured output
3. **[docs/FINDINGS.md](docs/FINDINGS.md)** — corrections to the original brief + source analysis
4. **[porting/PLAN.md](porting/PLAN.md)** — 8-step incremental port plan
5. **[porting/ops-inventory.md](porting/ops-inventory.md)** — all 22 ops: names, args, categories

---

## License

Apache-2.0 (same as Obscura upstream)
