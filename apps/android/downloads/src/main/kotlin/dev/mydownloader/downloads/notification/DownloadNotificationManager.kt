package dev.fetchx.downloads.notification

import android.app.Notification
import android.content.Context
import androidx.core.app.NotificationChannelCompat
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import dev.fetchx.core.download.DownloadItem
import dev.fetchx.core.download.DownloadState

/**
 * Creates and updates Android notifications for FetchX downloads.
 *
 * Three channels:
 *  - [CHANNEL_ACTIVE]   low-importance ongoing progress (silent)
 *  - [CHANNEL_COMPLETE] default-importance completion alerts
 *  - [CHANNEL_FAILED]   default-importance failure alerts
 *
 * Privacy: filenames are shown; URLs, credentials, and tokens are never included.
 * Throttling: callers are responsible for not calling notifyProgress on every byte.
 */
class DownloadNotificationManager(private val context: Context) {

    companion object {
        const val CHANNEL_ACTIVE   = "fetchx_downloads_active"
        const val CHANNEL_COMPLETE = "fetchx_downloads_complete"
        const val CHANNEL_FAILED   = "fetchx_downloads_failed"
        const val FOREGROUND_SUMMARY_ID = 1_000_000
    }

    private val nm = NotificationManagerCompat.from(context)

    init { createChannels() }

    private fun createChannels() {
        nm.createNotificationChannel(
            NotificationChannelCompat.Builder(CHANNEL_ACTIVE, NotificationManagerCompat.IMPORTANCE_LOW)
                .setName("Active Downloads")
                .setDescription("Progress for ongoing downloads")
                .build()
        )
        nm.createNotificationChannel(
            NotificationChannelCompat.Builder(CHANNEL_COMPLETE, NotificationManagerCompat.IMPORTANCE_DEFAULT)
                .setName("Completed Downloads")
                .build()
        )
        nm.createNotificationChannel(
            NotificationChannelCompat.Builder(CHANNEL_FAILED, NotificationManagerCompat.IMPORTANCE_DEFAULT)
                .setName("Failed Downloads")
                .build()
        )
    }

    fun notifyProgress(item: DownloadItem) {
        val s = item.state as? DownloadState.Active ?: return
        val total = s.totalBytes
        val pct   = if (total != null && total > 0) ((s.downloadedBytes.toFloat() / total) * 100).toInt() else -1
        val speed = if (s.speedBytesPerSec > 0) " · ${formatDownloadSpeed(s.speedBytesPerSec)}" else ""
        val eta   = s.etaSeconds?.let { " · ${formatEta(it)} left" } ?: ""
        val text  = "Downloading$speed$eta"

        val b = NotificationCompat.Builder(context, CHANNEL_ACTIVE)
            .setSmallIcon(android.R.drawable.stat_sys_download)
            .setContentTitle(item.fileName)
            .setContentText(text)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setPriority(NotificationCompat.PRIORITY_LOW)
        if (pct >= 0) b.setProgress(100, pct, false).setSubText("$pct%")
        else          b.setProgress(0, 0, true)
        safeNotify(stableId(item.id), b.build())
    }

    fun notifyQueued(item: DownloadItem) {
        val b = NotificationCompat.Builder(context, CHANNEL_ACTIVE)
            .setSmallIcon(android.R.drawable.stat_sys_download_done)
            .setContentTitle(item.fileName)
            .setContentText("Queued")
            .setOngoing(true).setOnlyAlertOnce(true)
            .setPriority(NotificationCompat.PRIORITY_LOW)
        safeNotify(stableId(item.id), b.build())
    }

    fun notifyPaused(item: DownloadItem) {
        val b = NotificationCompat.Builder(context, CHANNEL_ACTIVE)
            .setSmallIcon(android.R.drawable.stat_sys_download_done)
            .setContentTitle(item.fileName)
            .setContentText("Paused")
            .setOngoing(false).setOnlyAlertOnce(true)
            .setPriority(NotificationCompat.PRIORITY_LOW)
        safeNotify(stableId(item.id), b.build())
    }

    fun notifyCompleted(item: DownloadItem) {
        cancel(item.id)
        val size = (item.state as? DownloadState.Completed)?.let { " · ${formatBytes(it.totalBytes)}" } ?: ""
        val b = NotificationCompat.Builder(context, CHANNEL_COMPLETE)
            .setSmallIcon(android.R.drawable.stat_sys_download_done)
            .setContentTitle(item.fileName)
            .setContentText("Download complete$size")
            .setAutoCancel(true)
            .setPriority(NotificationCompat.PRIORITY_DEFAULT)
        safeNotify(stableId(item.id), b.build())
    }

    fun notifyFailed(item: DownloadItem) {
        cancel(item.id)
        val reason = (item.state as? DownloadState.Failed)?.reason?.take(80) ?: "Unknown error"
        val b = NotificationCompat.Builder(context, CHANNEL_FAILED)
            .setSmallIcon(android.R.drawable.stat_notify_error)
            .setContentTitle(item.fileName)
            .setContentText("Download failed: $reason")
            .setAutoCancel(true)
            .setPriority(NotificationCompat.PRIORITY_DEFAULT)
        safeNotify(stableId(item.id), b.build())
    }

    fun notifyCancelled(item: DownloadItem) = cancel(item.id)

    fun cancel(downloadId: String) = nm.cancel(stableId(downloadId))
    fun cancelAll() = nm.cancelAll()

    fun buildForegroundSummary(activeCount: Int): Notification =
        NotificationCompat.Builder(context, CHANNEL_ACTIVE)
            .setSmallIcon(android.R.drawable.stat_sys_download)
            .setContentTitle("FetchX")
            .setContentText(if (activeCount == 1) "1 download in progress" else "$activeCount downloads in progress")
            .setOngoing(true)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .build()

    internal fun stableId(downloadId: String): Int = downloadId.hashCode()

    private fun safeNotify(id: Int, n: Notification) {
        try { nm.notify(id, n) } catch (_: SecurityException) { /* POST_NOTIFICATIONS not granted */ }
    }
}
