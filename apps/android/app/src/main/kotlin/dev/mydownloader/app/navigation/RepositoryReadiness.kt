package dev.fetchx.app.navigation

import dev.fetchx.core.download.DownloadRepository
import kotlinx.coroutines.delay

/**
 * Waits until the Android download bridge/repository becomes available,
 * mirroring the bounded-polling readiness philosophy already used by the
 * magnet submission path in [dev.fetchx.app.MainActivity].
 *
 * On a true cold start the bridge is not yet initialized when `AppNavigation`
 * composes, so [dev.fetchx.bridge.BridgeInitializer.repositoryOrNull] returns
 * null and a notification deep-link would otherwise resolve against a
 * not-yet-ready [dev.fetchx.core.download.NoOpDownloadRepository] — which
 * always reports "no torrent" — so `TorrentDetailScreen` never opens.
 *
 * This helper polls [repositoryProvider] every [pollIntervalMs] (cancellable)
 * and gives up after [timeoutMs], returning null instead of resolving an
 * empty snapshot at t=0.
 *
 * It is the single readiness wait shared by both the magnet `ACTION_VIEW`
 * path and the notification deep-link path — no second polling loop is
 * duplicated.
 *
 * @return the ready [DownloadRepository], or null if the bridge did not
 *   initialize within [timeoutMs] (caller should keep the user on Downloads).
 */
internal suspend fun awaitRepositoryReady(
    repositoryProvider: () -> DownloadRepository?,
    pollIntervalMs: Long = 500L,
    timeoutMs: Long = 30_000L,
): DownloadRepository? {
    var waited = 0L
    while (waited < timeoutMs) {
        repositoryProvider()?.let { return it }
        delay(pollIntervalMs)
        waited += pollIntervalMs
    }
    return null
}
