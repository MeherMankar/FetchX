package dev.fetchx.browser

/**
 * Pure-Kotlin tab state store, testable without Android.
 * TabManager delegates tab state tracking to this class.
 * No Android imports allowed here.
 */
class TabStore {
    private val _tabs = mutableListOf<TabState>()
    private var _activeTabId: String? = null

    fun allTabs(): List<TabState> = _tabs.toList()
    fun allTabsForSpace(spaceId: String): List<TabState> = _tabs.filter { it.spaceId == spaceId }
    fun activeTabId(): String? = _activeTabId
    fun activeTab(): TabState? = _tabs.find { it.id == _activeTabId }
    fun tabCount(): Int = _tabs.size
    fun findTab(id: String): TabState? = _tabs.find { it.id == id }

    fun addTab(state: TabState) {
        _tabs.add(state)
        _activeTabId = state.id
    }

    /** Removes the tab and returns the index it occupied (or -1 if not found). */
    fun removeTab(tabId: String): Int {
        val index = _tabs.indexOfFirst { it.id == tabId }
        if (index != -1) _tabs.removeAt(index)
        if (_activeTabId == tabId) {
            _activeTabId = _tabs.getOrNull(index)?.id ?: _tabs.lastOrNull()?.id
        }
        return index
    }

    fun setActive(tabId: String): Boolean {
        if (_tabs.none { it.id == tabId }) return false
        _activeTabId = tabId
        return true
    }

    fun updateTab(tabId: String, update: (TabState) -> TabState): Boolean {
        val index = _tabs.indexOfFirst { it.id == tabId }
        if (index == -1) return false
        _tabs[index] = update(_tabs[index])
        return true
    }

    fun clear() {
        _tabs.clear()
        _activeTabId = null
    }
}

