package com.webshot

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.view.Gravity
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast

class MainActivity : Activity() {

    private lateinit var statusText: TextView

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        statusText = TextView(this).apply {
            text = "WebShot — tap Start to launch service"
            textSize = 16f
            gravity = Gravity.CENTER
            setPadding(32, 64, 32, 32)
        }

        val startBtn = Button(this).apply {
            text = "Start Service"
            setOnClickListener {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    if (checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)
                        != PackageManager.PERMISSION_GRANTED) {
                        requestPermissions(
                            arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1
                        )
                    }
                }
                val intent = Intent(this@MainActivity, ShotService::class.java)
                startForegroundService(intent)
                statusText.text = "Service started — POST to http://127.0.0.1:8899/shot"
                Toast.makeText(this@MainActivity, "Service started", Toast.LENGTH_SHORT).show()
            }
        }

        val stopBtn = Button(this).apply {
            text = "Stop Service"
            setOnClickListener {
                stopService(Intent(this@MainActivity, ShotService::class.java))
                statusText.text = "Service stopped"
                Toast.makeText(this@MainActivity, "Service stopped", Toast.LENGTH_SHORT).show()
            }
        }

        val testBtn = Button(this).apply {
            text = "Quick Test (HN)"
            setOnClickListener {
                statusText.text = "Taking screenshot…"
                Thread {
                    try {
                        val json = """{"url":"https://news.ycombinator.com","full_page":true,"wait_ms":2000}"""
                        val conn = java.net.URL("http://127.0.0.1:8899/shot").openConnection() as java.net.HttpURLConnection
                        conn.requestMethod = "POST"
                        conn.setRequestProperty("Content-Type", "application/json")
                        conn.doOutput = true
                        conn.outputStream.use { it.write(json.toByteArray()) }
                        val code = conn.responseCode
                        if (code == 200) {
                            val bytes = conn.inputStream.use { it.readBytes() }
                            val out = java.io.File(getExternalFilesDir(null), "webshot-test.png")
                            out.writeBytes(bytes)
                            runOnUiThread {
                                statusText.text = "Saved to ${out.absolutePath} (${bytes.size} bytes)"
                            }
                        } else {
                            val err = conn.errorStream?.bufferedReader()?.use { it.readText() } ?: "no error body"
                            runOnUiThread { statusText.text = "Error $code: $err" }
                        }
                        conn.disconnect()
                    } catch (e: Exception) {
                        runOnUiThread { statusText.text = "Error: ${e.message}" }
                    }
                }.start()
            }
        }

        val layout = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.CENTER
            setPadding(48, 96, 48, 96)
            addView(statusText)
            addView(startBtn)
            addView(stopBtn)
            addView(testBtn)
        }

        setContentView(layout)
    }
}
