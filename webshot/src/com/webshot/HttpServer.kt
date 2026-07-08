package com.webshot

import android.util.Log
import org.json.JSONObject
import java.io.BufferedReader
import java.io.InputStream
import java.io.InputStreamReader
import java.io.OutputStream
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

class HttpServer(
    private val port: Int,
    private val capturer: Capturer
) {
    companion object {
        private const val TAG = "WebShot/HTTP"
    }

    @Volatile private var running = false
    private var serverSocket: ServerSocket? = null
    private val executor = Executors.newSingleThreadExecutor()

    fun start() {
        running = true
        serverSocket = ServerSocket(port, 10, InetAddress.getByName("127.0.0.1"))
        Log.i(TAG, "Listening on 127.0.0.1:$port")
        executor.execute { acceptLoop() }
    }

    fun stop() {
        running = false
        try { serverSocket?.close() } catch (_: Exception) {}
        executor.shutdown()
        executor.awaitTermination(2, TimeUnit.SECONDS)
        Log.i(TAG, "Server stopped")
    }

    private fun acceptLoop() {
        while (running) {
            val client = try {
                serverSocket?.accept() ?: break
            } catch (e: Exception) {
                if (running) Log.e(TAG, "Accept error", e)
                break
            }
            try {
                handleClient(client)
            } catch (e: Exception) {
                Log.e(TAG, "Handler error", e)
            }
        }
    }

    private fun handleClient(socket: Socket) {
        socket.soTimeout = 40000
        val input = BufferedReader(InputStreamReader(socket.getInputStream(), "UTF-8"))
        val output = socket.getOutputStream()

        try {
            val requestLine = input.readLine() ?: return
            val parts = requestLine.split(" ")
            if (parts.size < 3) return
            val method = parts[0]
            val path = parts[1]

            val headers = mutableMapOf<String, String>()
            var line: String?
            while (true) {
                line = input.readLine() ?: break
                if (line.isEmpty()) break
                val ci = line.indexOf(':')
                if (ci > 0) {
                    headers[line.substring(0, ci).trim().lowercase()] =
                        line.substring(ci + 1).trim()
                }
            }

            val contentLength = headers["content-length"]?.toIntOrNull() ?: 0
            var body: String? = null
            if (contentLength > 0) {
                val buf = CharArray(contentLength)
                var read = 0
                while (read < contentLength) {
                    val n = input.read(buf, read, contentLength - read)
                    if (n < 0) break
                    read += n
                }
                body = String(buf, 0, read)
            }

            when {
                method == "GET" && path == "/health" -> {
                    sendJson(output, 200, """{"status":"ok"}""")
                }

                method == "POST" && path == "/shot" -> {
                    handleShot(output, body)
                }

                else -> {
                    sendJson(output, 404, """{"error":"not found"}""")
                }
            }
        } finally {
            try { socket.close() } catch (_: Exception) {}
        }
    }

    private fun handleShot(output: OutputStream, body: String?) {
        try {
            val json = JSONObject(body ?: "{}")

            val url = if (json.has("url") && !json.isNull("url"))
                json.getString("url") else null
            val html = if (json.has("html") && !json.isNull("html"))
                json.getString("html") else null

            if (url == null && html == null) {
                sendJson(output, 400, """{"error":"missing 'url' or 'html'"}""")
                return
            }

            val width = json.optInt("width", 1080)
            val height = json.optInt("height", 1920)
            val fullPage = json.optBoolean("full_page", false)
            val waitMs = json.optLong("wait_ms", 1500)
            val userAgent = if (json.has("user_agent") && !json.isNull("user_agent"))
                json.getString("user_agent") else null

            Log.i(TAG, "Shot request: url=$url width=$width height=$height full=$fullPage wait=$waitMs")

            val png = capturer.capture(url, html, width, height, fullPage, waitMs, userAgent)

            val header = "HTTP/1.1 200 OK\r\n" +
                "Content-Type: image/png\r\n" +
                "Content-Length: ${png.size}\r\n" +
                "Connection: close\r\n\r\n"
            output.write(header.toByteArray(Charsets.UTF_8))
            output.write(png)
            output.flush()
            Log.i(TAG, "Sent ${png.size} bytes")
        } catch (e: Exception) {
            Log.e(TAG, "Shot error", e)
            val msg = e.message?.replace("\"", "\\\"") ?: "unknown"
            sendJson(output, 500, """{"error":"$msg"}""")
        }
    }

    private fun sendJson(output: OutputStream, code: Int, body: String) {
        val bytes = body.toByteArray(Charsets.UTF_8)
        val statusText = if (code == 200) "OK" else "Error"
        val header = "HTTP/1.1 $code $statusText\r\n" +
            "Content-Type: application/json\r\n" +
            "Content-Length: ${bytes.size}\r\n" +
            "Connection: close\r\n\r\n"
        output.write(header.toByteArray(Charsets.UTF_8))
        output.write(bytes)
        output.flush()
    }
}
