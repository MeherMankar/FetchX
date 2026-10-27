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
 * Also accepts EXTRA_MAGNET_URI to work around Android's am-start URI truncation
 * at '?' for opaque magnet: URIs (useful for ADB test automation):
 *   adb shell am start -n dev.fetchx.app.debug/dev.fetchx.app.MainActivity \
 *       -a android.intent.action.VIEW \
 *       --es dev.fetchx.MAGNET_URI "magnet:?xt=urn:btih:..."
 *
 * Also handles ACTION_OPEN_DOWNLOAD intents fired by notification taps:
 *   Intent(DownloadNotificationManager.ACTION_OPEN_DOWNLOAD)
 *       .putExtra(DownloadNotificationManager.EXTRA_DOWNLOAD_ID, id)
 */
class MainActivity : ComponentActivity() {

    companion object {
        const val EXTRA_MAGNET_URI = "dev.fetchx.MAGNET_URI"
        /** ADB test extra: stop seeding for the given download ID. */
        const val EXTRA_STOP_SEEDING_ID = "dev.fetchx.STOP_SEEDING_ID"
    }

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
        // EXTRA_STOP_SEEDING_ID: stop seeding for a download by ID (ADB test automation).
        val stopSeedingId = i?.getStringExtra(EXTRA_STOP_SEEDING_ID)
        if (stopSeedingId != null) {
            android.util.Log.i("FetchXApp", "handleIntent: EXTRA_STOP_SEEDING_ID=$stopSeedingId")
            stopSeedingWhenReady(stopSeedingId)
            return
        }
        // EXTRA_MAGNET_URI takes priority over ACTION_VIEW for opaque magnet: URIs.
        val magnetExtra = i?.getStringExtra(EXTRA_MAGNET_URI)
        if (magnetExtra != null) {
            android.util.Log.i("FetchXApp", "handleIntent: EXTRA_MAGNET_URI=$magnetExtra")
            submitMagnetWhenReady(magnetExtra)
            return
        }
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

    private fun stopSeedingWhenReady(downloadId: String) {
        MainScope().launch {
            val repo = awaitRepositoryReady(repositoryProvider = {
                dev.fetchx.bridge.BridgeInitializer.repositoryOrNull()
            })
            if (repo is dev.fetchx.core.download.TorrentDownloadRepository) {
                android.util.Log.i("FetchXApp", "stopSeeding: bridge ready, stopping id=$downloadId")
                try {
                    repo.stopTorrentSeeding(downloadId)
                    android.util.Log.i("FetchXApp", "stopSeeding: success id=$downloadId")
                } catch (e: Exception) {
                    android.util.Log.e("FetchXApp", "stopSeeding: failed id=$downloadId: $e")
                }
            } else {
                android.util.Log.e("FetchXApp", "stopSeeding: bridge not ready after 30s")
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

