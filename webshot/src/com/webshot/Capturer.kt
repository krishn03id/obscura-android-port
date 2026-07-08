package com.webshot

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.view.View
import android.webkit.WebChromeClient
import android.webkit.WebView
import android.webkit.WebViewClient
import java.io.ByteArrayOutputStream
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference

class Capturer(private val context: Context) {

    companion object {
        private const val TAG = "WebShot"
        private const val MAX_HEIGHT = 10000
    }

    private val mainHandler = Handler(Looper.getMainLooper())

    @SuppressLint("SetJavaScriptEnabled")
    fun capture(
        url: String?,
        html: String?,
        width: Int,
        height: Int,
        fullPage: Boolean,
        waitMs: Long,
        userAgent: String?
    ): ByteArray {
        val result = AtomicReference<ByteArray?>(null)
        val error = AtomicReference<Throwable?>(null)
        val pageFinished = AtomicBoolean(false)
        val webViewRef = AtomicReference<WebView?>(null)

        // Step 1: Create WebView on main thread and start loading
        val createLatch = CountDownLatch(1)
        mainHandler.post {
            try {
                val webView = WebView(context)

                val settings = webView.settings
                settings.javaScriptEnabled = true
                settings.domStorageEnabled = true
                settings.loadWithOverviewMode = true
                settings.useWideViewPort = true
                settings.mediaPlaybackRequiresUserGesture = false
                settings.cacheMode = android.webkit.WebSettings.LOAD_DEFAULT
                if (userAgent != null) {
                    settings.userAgentString = userAgent
                }

                webView.setLayerType(View.LAYER_TYPE_SOFTWARE, null)

                webView.layout(0, 0, width, height)
                webView.measure(
                    View.MeasureSpec.makeMeasureSpec(width, View.MeasureSpec.EXACTLY),
                    View.MeasureSpec.makeMeasureSpec(height, View.MeasureSpec.EXACTLY)
                )
                webView.layout(0, 0, width, height)

                webView.webViewClient = object : WebViewClient() {
                    override fun onPageFinished(view: WebView?, url: String?) {
                        Log.d(TAG, "onPageFinished: $url")
                        pageFinished.set(true)
                    }

                    override fun onReceivedError(
                        view: WebView?,
                        request: android.webkit.WebResourceRequest?,
                        error: android.webkit.WebResourceError?
                    ) {
                        Log.w(TAG, "Resource error: ${error?.description}")
                    }
                }

                webView.webChromeClient = WebChromeClient()

                if (url != null) {
                    Log.d(TAG, "Loading URL: $url")
                    webView.loadUrl(url)
                } else if (html != null) {
                    Log.d(TAG, "Loading HTML (${html.length} chars)")
                    webView.loadDataWithBaseURL(
                        "about:blank", html, "text/html", "UTF-8", null
                    )
                }

                webViewRef.set(webView)
            } catch (e: Exception) {
                Log.e(TAG, "WebView creation error", e)
                error.set(e)
            }
            createLatch.countDown()
        }

        if (!createLatch.await(5, TimeUnit.SECONDS)) {
            throw RuntimeException("Timed out creating WebView")
        }
        error.get()?.let { throw RuntimeException("Failed to create WebView", it) }

        // Step 2: Wait for onPageFinished (on this thread — main thread runs WebView)
        var waited = 0L
        val maxLoadWait = 25000L
        while (!pageFinished.get() && waited < maxLoadWait) {
            Thread.sleep(100)
            waited += 100
        }
        if (!pageFinished.get()) {
            Log.w(TAG, "onPageFinished not received after ${maxLoadWait}ms, proceeding anyway")
        }

        // Step 3: Settle delay for JS/lazy content
        Log.d(TAG, "Settling for ${waitMs}ms")
        Thread.sleep(waitMs)

        // Step 4: Capture on main thread
        val captureLatch = CountDownLatch(1)
        mainHandler.post {
            val webView = webViewRef.get()
            if (webView == null) {
                error.set(RuntimeException("WebView was null"))
                captureLatch.countDown()
                return@post
            }

            try {
                val captureHeight = if (fullPage) {
                    val ch = webView.contentHeight
                    Log.d(TAG, "contentHeight = $ch, viewport height = $height")
                    when {
                        ch > MAX_HEIGHT -> MAX_HEIGHT
                        ch > height -> ch
                        else -> height
                    }
                } else {
                    height
                }

                val doCapture = Runnable {
                    try {
                        val w = maxOf(width, 1)
                        val h = maxOf(captureHeight, 1)
                        val bitmap = Bitmap.createBitmap(w, h, Bitmap.Config.ARGB_8888)
                        val canvas = Canvas(bitmap)
                        canvas.drawColor(Color.WHITE)
                        webView.draw(canvas)

                        val stream = ByteArrayOutputStream()
                        bitmap.compress(Bitmap.CompressFormat.PNG, 100, stream)
                        val bytes = stream.toByteArray()
                        Log.d(TAG, "Captured ${w}x${h} → ${bytes.size} bytes")
                        result.set(bytes)
                        bitmap.recycle()
                    } catch (e: Exception) {
                        Log.e(TAG, "Draw error", e)
                        error.set(e)
                    } finally {
                        try { webView.destroy() } catch (_: Exception) {}
                        captureLatch.countDown()
                    }
                }

                if (fullPage && captureHeight != height) {
                    Log.d(TAG, "Resizing WebView to ${width}x${captureHeight} for full-page")
                    webView.measure(
                        View.MeasureSpec.makeMeasureSpec(width, View.MeasureSpec.EXACTLY),
                        View.MeasureSpec.makeMeasureSpec(captureHeight, View.MeasureSpec.EXACTLY)
                    )
                    webView.layout(0, 0, width, captureHeight)
                    mainHandler.postDelayed(doCapture, 250)
                } else {
                    doCapture.run()
                }
            } catch (e: Exception) {
                Log.e(TAG, "Capture setup error", e)
                error.set(e)
                try { webView.destroy() } catch (_: Exception) {}
                captureLatch.countDown()
            }
        }

        if (!captureLatch.await(15, TimeUnit.SECONDS)) {
            mainHandler.post {
                try { webViewRef.get()?.destroy() } catch (_: Exception) {}
            }
            throw RuntimeException("Capture timed out after 15s")
        }

        error.get()?.let { throw RuntimeException("Capture failed: ${it.message}", it) }
        return result.get() ?: throw RuntimeException("Capture returned no data")
    }
}
