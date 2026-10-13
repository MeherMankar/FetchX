package dev.fetchx.browser

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import java.net.URI
import java.util.Locale

private const val MAX_RECORDED_HOSTS_PER_TAB = 20

data class TrackerTabStats(
    val blockedRequests: Long = 0,
    val blockedHosts: List<String> = emptyList(),
)

data class TrackerBlockingState(
    val enabled: Boolean = true,
    val tabStats: Map<String, TrackerTabStats> = emptyMap(),
)

/**
 * Keeps bounded, in-memory tracker blocking state separate from GeckoView and Compose.
 *
 * Only normalized hostnames are retained; request paths, query strings, and other
 * request data are discarded immediately.
 */
class TrackerBlockingController {
    private val lock = Any()
    private val openTabs = mutableSetOf<String>()
    private val hostsByTab = mutableMapOf<String, LinkedHashSet<String>>()
    private val countsByTab = mutableMapOf<String, Long>()
    private val _state = MutableStateFlow(TrackerBlockingState())

    val state: StateFlow<TrackerBlockingState> = _state.asStateFlow()

    @Volatile
    var onStateChanged: (() -> Unit)? = null

    fun registerTab(tabId: String) {
        if (tabId.isBlank()) return
        synchronized(lock) {
            openTabs.add(tabId)
        }
    }

    fun setEnabled(enabled: Boolean) {
        val changed = synchronized(lock) {
            if (_state.value.enabled == enabled) {
                false
            } else {
                publishLocked(enabled)
                true
            }
        }
        if (changed) onStateChanged?.invoke()
    }

    /**
     * Records a GeckoView-confirmed blocked request. Returns false if protection
     * is disabled or the callback does not identify a valid tab.
     */
    fun recordBlocked(tabId: String, requestUri: String): Boolean {
        if (tabId.isBlank()) return false
        val changed = synchronized(lock) {
            if (!_state.value.enabled || tabId !in openTabs) {
                false
            } else {
                val count = countsByTab[tabId] ?: 0
                countsByTab[tabId] = incrementSaturatedCount(count)

                val host = normalizedHost(requestUri)
                if (host != null) {
                    val hosts = hostsByTab.getOrPut(tabId, ::LinkedHashSet)
                    hosts.add(host)
                    while (hosts.size > MAX_RECORDED_HOSTS_PER_TAB) {
                        hosts.remove(hosts.first())
                    }
                }
                publishLocked(_state.value.enabled)
                true
            }
        }
        if (changed) onStateChanged?.invoke()
        return changed
    }

    fun removeTab(tabId: String) {
        val changed = synchronized(lock) {
            val wasOpen = openTabs.remove(tabId)
            val hadStats = countsByTab.remove(tabId) != null
            hostsByTab.remove(tabId)
            val removed = wasOpen || hadStats
            if (removed) publishLocked(_state.value.enabled)
            removed
        }
        if (changed) onStateChanged?.invoke()
    }

    fun clearTabs() {
        val changed = synchronized(lock) {
            if (openTabs.isEmpty() && countsByTab.isEmpty() && hostsByTab.isEmpty()) {
                false
            } else {
                openTabs.clear()
                countsByTab.clear()
                hostsByTab.clear()
                publishLocked(_state.value.enabled)
                true
            }
        }
        if (changed) onStateChanged?.invoke()
    }

    private fun publishLocked(enabled: Boolean) {
        _state.value = TrackerBlockingState(
            enabled = enabled,
            tabStats = countsByTab.mapValues { (tabId, count) ->
                TrackerTabStats(
                    blockedRequests = count,
                    blockedHosts = hostsByTab[tabId]?.toList().orEmpty(),
                )
            },
        )
    }
}

internal fun incrementSaturatedCount(count: Long): Long =
    if (count == Long.MAX_VALUE) count else count + 1

internal fun recordTrackerBlockedEvent(
    controller: TrackerBlockingController,
    tabId: String,
    isBlocking: Boolean,
    antiTrackingCategory: Int,
    requestUri: String,
): Boolean =
    isBlocking &&
        antiTrackingCategory and trackerAntiTrackingCategories(enabled = true) != 0 &&
        controller.recordBlocked(tabId, requestUri)

internal fun normalizedHost(requestUri: String): String? {
    return try {
        val uri = URI(requestUri)
        if (!uri.scheme.equals("http", ignoreCase = true) &&
            !uri.scheme.equals("https", ignoreCase = true)
        ) {
            null
        } else {
            uri.host
                ?.trimEnd('.')
                ?.takeIf(String::isNotBlank)
                ?.lowercase(Locale.ROOT)
        }
    } catch (_: IllegalArgumentException) {
        null
    } catch (_: java.net.URISyntaxException) {
        null
    }
}
