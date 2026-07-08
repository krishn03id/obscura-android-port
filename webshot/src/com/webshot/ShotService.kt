package com.webshot

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.util.Log

class ShotService : Service() {

    companion object {
        private const val TAG = "WebShot/Service"
        private const val CHANNEL_ID = "webshot_channel"
        private const val NOTIF_ID = 8899
    }

    private var server: HttpServer? = null
    private var capturer: Capturer? = null

    override fun onCreate() {
        super.onCreate()
        createNotificationChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val notification = Notification.Builder(this, CHANNEL_ID)
            .setContentTitle("WebShot")
            .setContentText("Screenshot service running on :8899")
            .setSmallIcon(android.R.drawable.ic_menu_camera)
            .setOngoing(true)
            .build()

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(NOTIF_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
        } else {
            startForeground(NOTIF_ID, notification)
        }

        if (capturer == null) {
            capturer = Capturer(this)
            server = HttpServer(8899, capturer!!)
            server?.start()
            Log.i(TAG, "Service started, server listening on 8899")
        }

        return START_STICKY
    }

    override fun onDestroy() {
        Log.i(TAG, "Service destroyed")
        server?.stop()
        server = null
        capturer = null
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null

    private fun createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(
                CHANNEL_ID,
                "WebShot Service",
                NotificationManager.IMPORTANCE_LOW
            ).apply {
                description = "WebView screenshot service"
            }
            val nm = getSystemService(NotificationManager::class.java)
            nm.createNotificationChannel(channel)
        }
    }
}
