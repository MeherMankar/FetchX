package dev.fetchx.app

import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import dev.fetchx.app.navigation.AppNavigation
import dev.fetchx.app.ui.theme.FetchXTheme

/**
 * Single-Activity entry point for FetchX Android.
 * Navigation is handled entirely by NavHost (Compose Navigation).
 *
 * Handles ACTION_VIEW intents so ADB (and other apps) can navigate the
 * browser to a URL:
 *   adb shell am start -n dev.fetchx.app.debug/dev.fetchx.app.MainActivity \
 *       -a android.intent.action.VIEW -d "https://example.com"
 */
class MainActivity : ComponentActivity() {

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        val initialUrl = intent?.takeIf { it.action == Intent.ACTION_VIEW }
            ?.dataString
        setContent {
            FetchXTheme {
                AppNavigation(initialUrl = initialUrl)
            }
        }
    }
}

