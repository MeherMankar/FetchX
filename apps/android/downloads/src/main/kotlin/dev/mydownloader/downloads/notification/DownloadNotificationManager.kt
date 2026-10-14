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
internal fun notificationTitle(fileName: String, showFilename: Boolean, fallback: String): String =
    if (showFilename) fileName else fallback

class DownloadNotificationManager(
    private val context: Context,
    private val privacySettings: DownloadNotificationPrivacySettings = DownloadNotificationPrivacySettings(context),
) {

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
            .setContentTitle(notificationTitle(item, "Download in progress"))
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
            .setContentTitle(notificationTitle(item, "Download queued"))
            .setContentText("Queued")
            .setOngoing(true).setOnlyAlertOnce(true)
            .setPriority(NotificationCompat.PRIORITY_LOW)
        safeNotify(stableId(item.id), b.build())
    }

    fun notifyPaused(item: DownloadItem) {
        val b = NotificationCompat.Builder(context, CHANNEL_ACTIVE)
            .setSmallIcon(android.R.drawable.stat_sys_download_done)
            .setContentTitle(notificationTitle(item, "Download paused"))
            .setContentText("Paused")
            .setOngoing(false).setOnlyAlertOnce(true)
            .setPriority(NotificationCompat.PRIORITY_LOW)
        safeNotify(stableId(item.id), b.build())
    }

    fun notifyCompleted(item: DownloadItem) {
        cancel(item.id)
        val size = (item.state as? DownloadState.Completed)?.let { " · ${formatBytes(it.totalBytes)}" } ?: ""
        val genericTitle = "Download completed"
        val b = NotificationCompat.Builder(context, CHANNEL_COMPLETE)
            .setSmallIcon(android.R.drawable.stat_sys_download_done)
            .setContentTitle(notificationTitle(item, genericTitle))
            .setContentText(if (privacySettings.showDownloadFilenamesInNotifications()) "Download complete$size" else "Download completed")
            .setAutoCancel(true)
            .setPriority(NotificationCompat.PRIORITY_DEFAULT)
        safeNotify(stableId(item.id), b.build())
    }

    fun notifyFailed(item: DownloadItem) {
        cancel(item.id)
        val reason = (item.state as? DownloadState.Failed)?.reason?.take(80) ?: "Unknown error"
        val b = NotificationCompat.Builder(context, CHANNEL_FAILED)
            .setSmallIcon(android.R.drawable.stat_notify_error)
            .setContentTitle(notificationTitle(item, "Download failed"))
            .setContentText(if (privacySettings.showDownloadFilenamesInNotifications()) "Download failed: $reason" else "Download failed")
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

    private fun notificationTitle(item: DownloadItem, fallback: String): String =
        notificationTitle(item.fileName, privacySettings.showDownloadFilenamesInNotifications(), fallback)

    private fun safeNotify(id: Int, n: Notification) {
        try { nm.notify(id, n) } catch (_: SecurityException) { /* POST_NOTIFICATIONS not granted */ }
    }
}

class DownloadNotificationPrivacySettings(
    private val context: Context,
) {
    companion object {
        private const val PREFS_NAME = "fetchx_notification_privacy"
        private const val KEY_SHOW_DOWNLOAD_FILENAMES = "show_download_filenames_in_notifications"
    }

    private val prefs by lazy { context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE) }

    fun showDownloadFilenamesInNotifications(): Boolean =
        prefs.getBoolean(KEY_SHOW_DOWNLOAD_FILENAMES, true)

    fun setShowDownloadFilenamesInNotifications(enabled: Boolean) {
        prefs.edit().putBoolean(KEY_SHOW_DOWNLOAD_FILENAMES, enabled).apply()
    }
}
