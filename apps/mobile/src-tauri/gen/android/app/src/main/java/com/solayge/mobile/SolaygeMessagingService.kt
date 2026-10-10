package com.solayge.mobile

import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage

/**
 * Receives background pushes. A message carries only opaque identifiers; the
 * title/body are generic, so a lockscreen preview never reveals agent content.
 *
 * While the app is in the foreground the webview already raises an in-app alert
 * from its own refresh, so this service stays quiet to avoid a duplicate. When
 * the app is backgrounded or not running, it posts a system notification.
 */
class SolaygeMessagingService : FirebaseMessagingService() {
    override fun onNewToken(token: String) {
        SolaygePush.saveToken(applicationContext, token)
    }

    override fun onMessageReceived(message: RemoteMessage) {
        val data = message.data
        val eventId = data["event_id"] ?: return
        if (SolaygePush.foreground) return
        if (!SolaygePush.markSeen(applicationContext, eventId)) return
        val title = message.notification?.title ?: titleFor(data["kind"])
        val body = message.notification?.body ?: "Open Solayge for details."
        SolaygePush.show(applicationContext, title, body, data)
    }

    private fun titleFor(kind: String?): String = when (kind) {
        "question" -> "Agent has a question"
        "approval" -> "Permission needed"
        "task_completed" -> "Task completed"
        "task_failed" -> "Task failed"
        else -> "Solayge"
    }
}
