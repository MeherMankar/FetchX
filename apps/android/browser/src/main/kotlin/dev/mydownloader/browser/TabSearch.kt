package dev.fetchx.browser

internal fun filterVisibleTabs(
    tabs: List<TabState>,
    searchQuery: String,
    filterSpaceId: String?,
): List<TabState> = tabs.filter { tab ->
    val matchesSpace = filterSpaceId == null || tab.spaceId == filterSpaceId
    val matchesQuery = searchQuery.isBlank() ||
        tab.title.orEmpty().contains(searchQuery, ignoreCase = true) ||
        tab.url.contains(searchQuery, ignoreCase = true)
    matchesSpace && matchesQuery
}
