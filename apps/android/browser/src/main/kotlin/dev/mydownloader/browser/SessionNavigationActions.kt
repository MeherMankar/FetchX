package dev.fetchx.browser

internal object SessionNavigationActions {
    fun goBackIfAvailable(tab: TabState?, goBack: () -> Unit): Boolean {
        if (tab?.canGoBack != true) return false
        goBack()
        return true
    }

    fun goForwardIfAvailable(tab: TabState?, goForward: () -> Unit): Boolean {
        if (tab?.canGoForward != true) return false
        goForward()
        return true
    }
}
