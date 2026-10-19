package dev.fetchx.app

import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.runtime.mutableStateOf
import dev.fetchx.app.navigation.AppNavigation
import dev.fetchx.app.ui.theme.FetchXTheme
import dev.fetchx.app.navigation.awaitRepositoryReady
import dev.fetchx.downloads.notification.DownloadNotificationManager
import kotlinx.coroutines.MainScope
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
 *
 * Also handles ACTION_OPEN_DOWNLOAD intents fired by notification taps:
 *   Intent(DownloadNotificationManager.ACTION_OPEN_DOWNLOAD)
 *       .putExtra(DownloadNotificationManager.EXTRA_DOWNLOAD_ID, id)
 */
class MainActivity : ComponentActivity() {

    // Shared state that AppNavigation observes for new intents arriving
    // via onNewIntent (when the activity is already running).
    private val pendingIntentUrl = mutableStateOf<String?>(null)

    // Shared state for notification deep-links: carries the download ID to open.
    private val pendingDownloadId = mutableStateOf<String?>(null)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()

        handleIncomingIntent(intent)

        setContent {
            FetchXTheme {
                AppNavigation(
                    initialUrl = intent?.takeIf { it.action == Intent.ACTION_VIEW }?.dataString
                        ?.takeUnless { it.startsWith("magnet:", ignoreCase = true) },
                    pendingIntentUrl = pendingIntentUrl,
                    pendingDownloadId = pendingDownloadId,
                )
            }
        }
    }

    /**
     * Dispatches an incoming intent to the correct pending-state bucket so that
     * AppNavigation can observe and react to it without coupling to Android lifecycle.
     */
    private fun handleIncomingIntent(i: Intent?) {
        when (i?.action) {
            DownloadNotificationManager.ACTION_OPEN_DOWNLOAD -> {
                val id = i.getStringExtra(DownloadNotificationManager.EXTRA_DOWNLOAD_ID)
                if (id != null) {
                    android.util.Log.i("FetchXApp", "handleIntent: open download id=$id")
                    pendingDownloadId.value = id
                }
            }
            Intent.ACTION_VIEW -> {
                val url = i.dataString ?: return
                android.util.Log.i("FetchXApp", "handleIntent: ACTION_VIEW url=$url")
                pendingIntentUrl.value = url
                if (url.startsWith("magnet:", ignoreCase = true)) {
                    submitMagnetWhenReady(url)
                }
            }
        }
    }

    private fun submitMagnetWhenReady(magnetUri: String) {
        MainScope().launch {
            // Wait up to 30s for the bridge/repository to be ready, reusing the
            // same bounded-readiness wait as the notification deep-link path.
            val repo = awaitRepositoryReady(repositoryProvider = {
                dev.fetchx.bridge.BridgeInitializer.repositoryOrNull()
            })
            if (repo is dev.fetchx.core.download.TorrentDownloadRepository) {
                android.util.Log.i("FetchXApp", "submitMagnet: bridge ready, adding magnet")
                try {
                    val id = repo.addMagnet(magnetUri, dev.fetchx.core.download.DownloadPriority.DEFAULT)
                    android.util.Log.i("FetchXApp", "submitMagnet: added id=$id")
                } catch (e: Exception) {
                    android.util.Log.e("FetchXApp", "submitMagnet: failed: $e")
                }
            } else {
                android.util.Log.e("FetchXApp", "submitMagnet: bridge not ready after 30s")
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handleIncomingIntent(intent)
    }
}

