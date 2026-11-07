package dev.fetchx.browser

import dev.fetchx.browser.TabState

internal class TabRestoreGuard {
    private var revision = 0L
    private var disposed = false

    @Synchronized
    fun version(): Long = revision

    @Synchronized
    fun markStateChanged() {
        revision += 1
    }

    @Synchronized
    fun canApply(version: Long): Boolean = !disposed && revision == version

    @Synchronized
    fun isDisposed(): Boolean = disposed

    @Synchronized
    fun dispose() {
        disposed = true
    }
}

internal class TabSnapshotWriteGate {
    private var restoring = false
    private var pendingSnapshot: Pair<List<TabState>, String?>? = null
    private var disposed = false

    fun beginRestore() {
        check(!disposed)
        restoring = true
        pendingSnapshot = null
    }

    fun submit(
        tabs: List<TabState>,
        activeId: String?,
        persist: (List<TabState>, String?) -> Unit,
    ) {
        if (disposed) return
        if (restoring) {
            pendingSnapshot = tabs.toList() to activeId
        } else {
            persist(tabs, activeId)
        }
    }

    fun completeRestore(persist: (List<TabState>, String?) -> Unit) {
        if (disposed) return
        restoring = false
        pendingSnapshot?.let { (tabs, activeId) ->
            pendingSnapshot = null
            persist(tabs, activeId)
        }
    }

    fun dispose() {
        disposed = true
        restoring = false
        pendingSnapshot = null
    }
}

internal class TabRestoreNavigationGate {
    private var restoring = false
    private var pendingUrl: String? = null
    private var disposed = false

    fun beginRestore() {
        check(!disposed)
        restoring = true
        pendingUrl = null
    }

    fun defer(url: String): Boolean {
        if (disposed || !restoring) return false
        pendingUrl = url
        return true
    }

    fun completeRestore(): String? {
        if (disposed) return null
        restoring = false
        return pendingUrl.also { pendingUrl = null }
    }

    fun dispose() {
        disposed = true
        restoring = false
        pendingUrl = null
    }
}
