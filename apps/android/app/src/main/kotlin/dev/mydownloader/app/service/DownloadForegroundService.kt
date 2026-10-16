package dev.fetchx.app.service

import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.util.Log
import dev.fetchx.bridge.BridgeInitializer
import dev.fetchx.core.download.DownloadState
import dev.fetchx.downloads.notification.DownloadNotificationManager
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.launchIn
import kotlinx.coroutines.flow.onEach
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.Job

private const val TAG = "DownloadForegroundSvc"

/**
 * Minimal foreground service that keeps the FetchX process alive during active downloads.
 *
 * Lifecycle:
 * - Started from [dev.fetchx.app.FetchXApp] when a download begins.
 * - Observes downloads via the existing bridge repository — no second download system.
 * - Stops itself when no active or queued downloads remain.
 * - Uses FOREGROUND_SERVICE_DATA_SYNC (declared in manifest).
 *
 * Architecture: this service owns NO download logic. All download operations
 * remain in the Rust core (QueueManager / Scheduler). This service only keeps
 * the process alive and updates the foreground notification.
 */
class DownloadForegroundService : Service() {

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private lateinit var notificationManager: DownloadNotificationManager
    private var downloadsJob: Job? = null

    override fun onCreate() {
        super.onCreate()
        notificationManager = DownloadNotificationManager(applicationContext)
        Log.i(TAG, "Service created")
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val repo = BridgeInitializer.repositoryOrNull()
        if (repo == null) {
            Log.w(TAG, "Bridge unavailable — stopping service")
            stopSelf(startId)
            return START_NOT_STICKY
        }

        if (downloadsJob?.isActive == true) return START_NOT_STICKY

        startForegroundCompat(0)
        downloadsJob = kotlinx.coroutines.flow.flow {
            repo.awaitInitialSnapshot()
            repo.observeDownloads().collect { emit(it) }
        }
            .onEach { items ->
                val activeCount = items.count { it.state is DownloadState.Active }
                val queuedCount = items.count { it.state is DownloadState.Queued }
                val runningCount = activeCount + queuedCount

                items.forEach { item ->
                    when (item.state) {
                        is DownloadState.Active -> notificationManager.notifyProgress(item)
                        is DownloadState.Queued -> notificationManager.notifyQueued(item)
                        is DownloadState.Paused -> notificationManager.notifyPaused(item)
                        is DownloadState.Completed -> notificationManager.notifyCompleted(item)
                        is DownloadState.Failed -> notificationManager.notifyFailed(item)
                        is DownloadState.Cancelled -> notificationManager.notifyCancelled(item)
                    }
                }

                if (runningCount == 0) {
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                        stopForeground(STOP_FOREGROUND_DETACH)
                    } else {
                        @Suppress("DEPRECATION")
                        stopForeground(true)
                    }
                    stopSelf(startId)
                    Log.i(TAG, "No active or queued downloads — stopping service")
                    return@onEach
                }

                startForegroundCompat(activeCount)
            }
            .catch { error ->
                Log.e(TAG, "Download observation failed", error)
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    stopForeground(STOP_FOREGROUND_REMOVE)
                } else {
                    @Suppress("DEPRECATION")
                    stopForeground(true)
                }
                stopSelf(startId)
            }
            .launchIn(scope)

        return START_NOT_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onDestroy() {
        scope.cancel()
        Log.i(TAG, "Service destroyed")
        super.onDestroy()
    }

    private fun startForegroundCompat(activeCount: Int) {
        val notification = notificationManager.buildForegroundSummary(activeCount)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(
                DownloadNotificationManager.FOREGROUND_SUMMARY_ID,
                notification,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC,
            )
        } else {
            startForeground(DownloadNotificationManager.FOREGROUND_SUMMARY_ID, notification)
        }
    }
}
