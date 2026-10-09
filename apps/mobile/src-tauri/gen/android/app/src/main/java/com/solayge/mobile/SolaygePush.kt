package com.solayge.mobile

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Bundle
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import org.json.JSONObject
import java.io.File

/**
 * Native push helpers shared by the FCM service and the activity.
 *
 * The FCM token and any notification tap are written as small JSON files under
 * the app's data directory, where the Rust shell reads them through the
 * `push_token` / `take_push_tap` commands. Only opaque identifiers are ever
 * stored or shown; prompts, code, and message content stay on the desktop and
 * are fetched over Iroh/Tailscale after the app reconnects.
 */
object SolaygePush {
    const val CHANNEL_ID = "solayge"
    const val CHANNEL_NAME = "Solayge alerts"

    private const val TOKEN_FILE = "push-token.json"
    private const val TAP_FILE = "push-tap.json"
    private const val PREFS = "solayge_push"
    private const val SEEN_KEY = "seen_events"
    private const val MAX_SEEN = 200

    /** Identifiers a push is allowed to carry. */
    private val TAP_KEYS = listOf("event_id", "kind", "desktop_id", "session_id", "approval_id")

    /** Whether the webview is currently in the foreground. Set by MainActivity. */
    @Volatile
    var foreground: Boolean = false

    /**
     * The directory Tauri exposes as the app data directory on Android.
     *
     * Tauri's Android path resolver returns `Context.getDataDir()` (the package
     * directory) for `app_data_dir()`, exactly as the Rust shell uses it for
     * `paired-desktops.json` and the other bridge files. Unlike the desktop
     * resolver, it does NOT append the bundle identifier, so writing into a
     * `<dataDir>/<packageName>` subdirectory would put these files somewhere the
     * Rust shell never reads. Keep the base directory.
     */
    private fun dataDir(context: Context): File {
        val dir = context.dataDir
        if (!dir.exists()) dir.mkdirs()
        return dir
    }

    /** Persist the FCM registration token for the Rust shell to register. */
    fun saveToken(context: Context, token: String) {
        val json = JSONObject()
            .put("token", token)
            .put("platform", "android")
            .put("updated_at", System.currentTimeMillis() / 1000)
        runCatching { File(dataDir(context), TOKEN_FILE).writeText(json.toString()) }
    }

    /** Persist a notification tap so the app can deep-link once it starts. */
    fun saveTap(context: Context, extras: Bundle?) {
        if (extras == null) return
        val json = JSONObject()
        for (key in TAP_KEYS) {
            extras.getString(key)?.let { json.put(key, it) }
        }
        if (json.length() == 0) return
        runCatching { File(dataDir(context), TAP_FILE).writeText(json.toString()) }
    }

    /** Create the notification channel on Android 8+ (idempotent). */
    fun ensureChannel(context: Context) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val manager = context.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        if (manager.getNotificationChannel(CHANNEL_ID) == null) {
            val channel = NotificationChannel(
                CHANNEL_ID,
                CHANNEL_NAME,
                NotificationManager.IMPORTANCE_HIGH,
            )
            channel.description = "Agent questions, task completions, and failures"
            manager.createNotificationChannel(channel)
        }
    }

    /**
     * Record an event id and report whether it is new. FCM can redeliver a
     * message, and a push may race the app's own local alert; this keeps the
     * user from seeing the same event twice.
     */
    fun markSeen(context: Context, eventId: String): Boolean {
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val seen = prefs.getStringSet(SEEN_KEY, emptySet())?.toMutableSet() ?: mutableSetOf()
        if (!seen.add(eventId)) return false
        val trimmed = if (seen.size > MAX_SEEN) seen.toList().takeLast(MAX_SEEN).toSet() else seen
        prefs.edit().putStringSet(SEEN_KEY, trimmed).apply()
        return true
    }

    /** Show one notification, opening the named session/approval when tapped. */
    fun show(context: Context, title: String, body: String, data: Map<String, String>) {
        ensureChannel(context)
        val intent = Intent(context, MainActivity::class.java).apply {
            flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
            for ((key, value) in data) putExtra(key, value)
        }
        val pendingFlags = PendingIntent.FLAG_UPDATE_CURRENT or
            (if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) PendingIntent.FLAG_IMMUTABLE else 0)
        val requestCode = (data["event_id"] ?: title).hashCode()
        val pending = PendingIntent.getActivity(context, requestCode, intent, pendingFlags)
        val notification = NotificationCompat.Builder(context, CHANNEL_ID)
            .setSmallIcon(R.mipmap.ic_launcher)
            .setContentTitle(title)
            .setContentText(body)
            .setAutoCancel(true)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setContentIntent(pending)
            .build()
        try {
            NotificationManagerCompat.from(context).notify(requestCode, notification)
        } catch (_: SecurityException) {
            // POST_NOTIFICATIONS not granted; nothing else to do.
        }
    }
}
