package dev.fetchx.app

import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.runtime.mutableStateOf
import dev.fetchx.app.navigation.AppNavigation
import dev.fetchx.app.ui.theme.FetchXTheme
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * Single-Activity entry point for FetchX Android.
 * Navigation is handled entirely by NavHost (Compose Navigation).
 *
 * Handles ACTION_VIEW intents so ADB (and other apps) can navigate the
 * browser to a URL or add a magnet: download:
 *   adb shell am start -n dev.fetchx.app.debug/dev.fetchx.app.MainActivity \
 *       -a android.intent.action.VIEW -d "https://example.com"
 *   adb shell am start -a android.intent.action.VIEW \
 *       -d "magnet:?xt=..." dev.fetchx.app.debug/dev.fetchx.app.MainActivity
 */
class MainActivity : ComponentActivity() {

    // Shared state that AppNavigation observes for new intents arriving
    // via onNewIntent (when the activity is already running).
    private val pendingIntentUrl = mutableStateOf<String?>(null)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        val initialUrl = intent?.takeIf { it.action == Intent.ACTION_VIEW }
            ?.dataString
        pendingIntentUrl.value = initialUrl

        // Direct magnet submission from onCreate (handles cold-start with magnet intent).
        if (initialUrl?.startsWith("magnet:", ignoreCase = true) == true) {
            android.util.Log.i("FetchXApp", "onCreate: magnet intent detected: $initialUrl")
            submitMagnetWhenReady(initialUrl)
        }

        setContent {
            FetchXTheme {
                AppNavigation(
                    initialUrl = initialUrl,
                    pendingIntentUrl = pendingIntentUrl,
                )
            }
        }
    }

    private fun submitMagnetWhenReady(magnetUri: String) {
        MainScope().launch {
            // Wait up to 30s for bridge to initialize, checking every 500ms.
            var waited = 0
            while (waited < 30_000) {
                val repo = dev.fetchx.bridge.BridgeInitializer.repositoryOrNull()
                if (repo is dev.fetchx.core.download.TorrentDownloadRepository) {
                    android.util.Log.i("FetchXApp", "submitMagnet: bridge ready, adding magnet")
                    try {
                        val id = repo.addMagnet(magnetUri, dev.fetchx.core.download.DownloadPriority.DEFAULT)
                        android.util.Log.i("FetchXApp", "submitMagnet: added id=$id")
                    } catch (e: Exception) {
                        android.util.Log.e("FetchXApp", "submitMagnet: failed: $e")
                    }
                    return@launch
                }
                kotlinx.coroutines.delay(500)
                waited += 500
            }
            android.util.Log.e("FetchXApp", "submitMagnet: bridge not ready after 30s")
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        val url = intent.takeIf { it.action == Intent.ACTION_VIEW }?.dataString
        if (url != null) {
            android.util.Log.i("FetchXApp", "onNewIntent: received URL: $url")
            pendingIntentUrl.value = url

            // For magnet links, also try direct submission via bridge as a fallback.
            if (url.startsWith("magnet:", ignoreCase = true)) {
                submitMagnetWhenReady(url)
            }
        }
    }
}

