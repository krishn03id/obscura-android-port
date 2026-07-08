Thought for 1sHere's the complete framework blueprint for the Android WebView screenshot helper. Hand this to your AI to implement.

---

## WebView Screenshot Service — Blueprint

**Goal:** A headless Android helper that takes a URL (or raw HTML), renders it in the system WebView (full Chromium: JS, flexbox, images, fonts), and outputs a PNG. Callable from Termux.

### Architecture

```plaintext
Termux (curl/CLI)
    │  HTTP POST {url | html, width, height, wait_ms, full_page}
    ▼
[1] Foreground Service + embedded HTTP server (NanoHTTPD, port 8899)
    │
    ▼
[2] Off-screen WebView (created on main thread, never attached to a visible Activity)
    │   - settings: javaScriptEnabled, domStorageEnabled, loadWithOverviewMode
    │   - layout(0, 0, width, height) + measure() to force render pass
    ▼
[3] Load: loadUrl(url) or loadDataWithBaseURL(html)
    │
    ▼
[4] Wait strategy:
    │   a. onPageFinished callback
    │   b. + settle delay (wait_ms, default 1500ms) for JS/lazy content
    │   c. optional: evaluateJavascript("document.readyState") polling
    ▼
[5] Capture:
    │   - Fixed viewport: Bitmap(width, height) + webView.draw(canvas)
    │   - Full page: measure content via evaluateJavascript(
    │       "document.documentElement.scrollHeight"), resize WebView layout
    │       to that height, then draw
    │   - API 26+: alternatively PixelCopy for hardware-accelerated surfaces
    ▼
[6] Compress: bitmap.compress(PNG, 100, stream) → return bytes as HTTP response
```

### Project structure

```plaintext
webshot/
├── app/src/main/
│   ├── AndroidManifest.xml        # INTERNET, FOREGROUND_SERVICE perms
│   ├── java/.../
│   │   ├── MainActivity.kt        # minimal UI: start/stop service, show port
│   │   ├── ShotService.kt         # foreground service, owns WebView + server
│   │   ├── HttpServer.kt          # NanoHTTPD, routes POST /shot, GET /health
│   │   └── Capturer.kt            # WebView setup, load, wait, draw-to-bitmap
│   └── res/...                    # notification channel strings
└── build.gradle.kts               # minSdk 26, no exotic deps (NanoHTTPD only)
```

### API contract

```plaintext
POST http://127.0.0.1:8899/shot
{
  "url": "https://example.com",     // OR "html": "<h1>hi</h1>"
  "width": 1080,                    // viewport px, default 1080
  "height": 1920,                   // ignored if full_page=true
  "full_page": true,
  "wait_ms": 1500,                  // settle time after onPageFinished
  "user_agent": "optional override"
}
→ 200 OK, Content-Type: image/png, body = PNG bytes
→ 400 bad request / 504 timeout (30s hard cap)

GET /health → {"status":"ok"}
```

### Termux usage

```shellscript
curl -s -X POST http://127.0.0.1:8899/shot \
  -H 'Content-Type: application/json' \
  -d '{"url":"https://news.ycombinator.com","full_page":true}' \
  -o shot.png
```

### Critical implementation gotchas (tell your AI these)

1. **WebView must be created and touched only on the main thread** — post all load/draw calls via `Handler(Looper.getMainLooper())`; the HTTP thread blocks on a `CountDownLatch`/`CompletableFuture` until capture finishes.
2. **Off-screen rendering:** call `webView.measure(makeMeasureSpec(width, EXACTLY), makeMeasureSpec(height, EXACTLY))` then `layout(0,0,w,h)` or `draw()` produces a blank bitmap.
3. **`onPageFinished` fires before JS apps paint** — the settle delay is mandatory; SPAs need 1500–4000ms.
4. **Serialize requests:** one WebView, one capture at a time (a simple mutex/queue) — parallel loads corrupt state.
5. **Full-page height:** get `scrollHeight` via `evaluateJavascript`, re-layout, wait one frame (`postDelayed 100ms`), then draw. Cap height (~10,000px) to avoid OOM on huge pages.
6. **Software layer for draw:** `webView.setLayerType(LAYER_TYPE_SOFTWARE, null)` if `draw()` yields blank on hardware-accelerated devices (or use `PixelCopy` with a `SurfaceView` path).
7. **Foreground service + notification** is required on Android 8+ or the OS kills the server; also request battery-optimization exemption for long-running use.
8. **Bind server to 127.0.0.1 only** — never 0.0.0.0, or any app/network peer can drive your WebView.
9. `android:usesCleartextTraffic="true"` if you want to screenshot plain-http sites.


### Non-goals (keep it simple)

No cookies/login management, no PDF output, no scrolling animations, no multi-tab — add later if needed.

---

This pairs with what we built: **webshot** for real-world sites, **obscura-render** (your pure-Rust engine) for fast rendering of HTML you control. Want me to also add this blueprint as a markdown file in the repo alongside `task.md` so it's versioned?
