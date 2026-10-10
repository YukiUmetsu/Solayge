package com.solayge.mobile

import android.content.Intent
import android.os.Bundle
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    // The channel must exist before any background notification arrives.
    SolaygePush.ensureChannel(this)
    // A launch from a notification tap: remember it for the app to consume.
    SolaygePush.saveTap(this, intent?.extras)
  }

  override fun onResume() {
    super.onResume()
    SolaygePush.foreground = true
  }

  override fun onPause() {
    SolaygePush.foreground = false
    super.onPause()
  }

  override fun onNewIntent(intent: Intent) {
    super.onNewIntent(intent)
    setIntent(intent)
    SolaygePush.saveTap(this, intent.extras)
  }
}
