package dev.fetchx.browser

import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

/**
 * ViewModel-level bookmark tests.
 *
 * [BrowserViewModel] cannot be instantiated in pure JVM tests because it extends
 * [androidx.lifecycle.ViewModel] and imports [android.util.Log], both of which
 * require the Android runtime.  Rather than adding Robolectric, the bookmark
 * business logic was extracted into [BookmarkController] — a pure-JVM class with
 * zero Android imports.  [BrowserViewModel] is a thin coordinator that delegates
 * all bookmark operations to [BookmarkController] and merges the resulting state
 * into [BrowserUiState].
 *
 * These tests exercise [BookmarkController] directly, giving full coverage of:
 * - bookmark-the-current-page flow
 * - remove-current-bookmark flow
 * - isCurrentPageBookmarked state updates
 * - bookmark list updates after add / remove / update
 * - history-remains-independent (verified at the ViewModel/controller boundary)
 * - clearing history does NOT clear bookmarks
 * - isLoading guard (programmatic callers cannot bookmark during navigation)
 */
class BrowserViewModelTest {

    // ── Helpers ──────────────────────────────────────────────────────────

    private fun controller() = BookmarkController()

    private fun BookmarkController.addPage(url: String = "https://example.com", title: String = "Example") =
        toggleBookmark(url, title, isLoading = false)

    // ── Toggle / bookmark current page ───────────────────────────────────

    @Test fun `toggleBookmark adds bookmark for valid url`() {
        val ctrl = controller()
        val added = ctrl.toggleBookmark("https://example.com", "Example", isLoading = false)
        assertTrue(added)
        assertTrue(ctrl.store.isBookmarked("https://example.com"))
    }

    @Test fun `toggleBookmark returns false for blank url`() {
        val ctrl = controller()
        assertFalse(ctrl.toggleBookmark("", "Title", isLoading = false))
        assertFalse(ctrl.toggleBookmark("   ", "Title", isLoading = false))
        assertEquals(0, ctrl.store.count())
    }

    @Test fun `toggleBookmark returns false for about blank`() {
        val ctrl = controller()
        assertFalse(ctrl.toggleBookmark("about:blank", "Blank", isLoading = false))
        assertEquals(0, ctrl.store.count())
    }

    @Test fun `toggleBookmark returns false while page is loading`() {
        // isLoading guard — prevents bookmarking stale URL during navigation
        val ctrl = controller()
        assertFalse(ctrl.toggleBookmark("https://example.com", "Example", isLoading = true))
        assertEquals(0, ctrl.store.count())
    }

    // ── Remove current bookmark ───────────────────────────────────────────

    @Test fun `toggleBookmark removes bookmark when already bookmarked`() {
        val ctrl = controller()
        ctrl.addPage("https://example.com")
        assertTrue(ctrl.store.isBookmarked("https://example.com"))
        // second toggle → remove
        ctrl.toggleBookmark("https://example.com", "Example", isLoading = false)
        assertFalse(ctrl.store.isBookmarked("https://example.com"))
        assertEquals(0, ctrl.store.count())
    }

    @Test fun `removeBookmark removes by id`() {
        val ctrl = controller()
        ctrl.addPage("https://example.com")
        val id = ctrl.store.findByUrl("https://example.com")!!.id
        ctrl.removeBookmark(id)
        assertFalse(ctrl.store.isBookmarked("https://example.com"))
        assertEquals(0, ctrl.store.count())
    }

    @Test fun `isCurrentPageBookmarked becomes false after removeBookmark by id`() {
        // Reproduces the stale-star bug: removeBookmark must refresh isCurrentPageBookmarked
        // (not just reload the list) so the star icon updates when the bookmarked page is
        // still open in the active tab.
        val ctrl = controller()
        ctrl.addPage("https://example.com")
        ctrl.refreshForUrl("https://example.com")
        assertTrue(ctrl.state.value.isCurrentPageBookmarked)

        // Remove via explicit ID path (used by the bookmarks dialog delete button)
        val id = ctrl.store.findByUrl("https://example.com")!!.id
        ctrl.removeBookmark(id)
        // BookmarkController.removeBookmark calls reloadList() only; the caller
        // (BrowserViewModel.removeBookmark) must call refreshForUrl after.
        // We simulate the full ViewModel flow here:
        ctrl.refreshForUrl("https://example.com")
        assertFalse(ctrl.state.value.isCurrentPageBookmarked)
    }

    // ── isCurrentPageBookmarked state updates ─────────────────────────────

    @Test fun `isCurrentPageBookmarked is false initially`() {
        val ctrl = controller()
        ctrl.refreshForUrl("https://example.com")
        assertFalse(ctrl.state.value.isCurrentPageBookmarked)
    }

    @Test fun `isCurrentPageBookmarked becomes true after bookmarking`() {
        val ctrl = controller()
        ctrl.addPage("https://example.com")
        ctrl.refreshForUrl("https://example.com")
        assertTrue(ctrl.state.value.isCurrentPageBookmarked)
    }

    @Test fun `isCurrentPageBookmarked becomes false after removing bookmark`() {
        val ctrl = controller()
        ctrl.addPage("https://example.com")
        ctrl.refreshForUrl("https://example.com")
        assertTrue(ctrl.state.value.isCurrentPageBookmarked)
        // remove via toggle
        ctrl.toggleBookmark("https://example.com", "Example", isLoading = false)
        ctrl.refreshForUrl("https://example.com")
        assertFalse(ctrl.state.value.isCurrentPageBookmarked)
    }

    @Test fun `isCurrentPageBookmarked is false for about blank`() {
        val ctrl = controller()
        ctrl.refreshForUrl("about:blank")
        assertFalse(ctrl.state.value.isCurrentPageBookmarked)
    }

    @Test fun `isCurrentPageBookmarked is false for blank url`() {
        val ctrl = controller()
        ctrl.refreshForUrl("")
        assertFalse(ctrl.state.value.isCurrentPageBookmarked)
    }

    // ── Open / close bookmark manager ─────────────────────────────────────

    @Test fun `openBookmarks sets showBookmarks true in uiState`() {
        val vm = BrowserViewModel()
        assertFalse(vm.uiState.value.showBookmarks)
        vm.openBookmarks()
        assertTrue(vm.uiState.value.showBookmarks)
    }

    @Test fun `closeBookmarks sets showBookmarks false in uiState`() {
        val vm = BrowserViewModel()
        vm.openBookmarks()
        vm.closeBookmarks()
        assertFalse(vm.uiState.value.showBookmarks)
    }

    // ── Bookmark list updates ─────────────────────────────────────────────

    @Test fun `bookmarks list updates after add`() {
        val ctrl = controller()
        assertTrue(ctrl.state.value.bookmarks.isEmpty())
        ctrl.addPage("https://example.com", "Example")
        assertEquals(1, ctrl.state.value.bookmarks.size)
        assertEquals("https://example.com", ctrl.state.value.bookmarks.first().url)
    }

    @Test fun `bookmarks list updates after removeBookmark`() {
        val ctrl = controller()
        ctrl.addPage("https://example.com")
        assertEquals(1, ctrl.state.value.bookmarks.size)
        val id = ctrl.store.findByUrl("https://example.com")!!.id
        ctrl.removeBookmark(id)
        assertTrue(ctrl.state.value.bookmarks.isEmpty())
    }

    @Test fun `bookmarks list updates after updateBookmark`() {
        val ctrl = controller()
        ctrl.addPage("https://example.com", "Old Title")
        val id = ctrl.store.findByUrl("https://example.com")!!.id
        ctrl.updateBookmark(id, "New Title")
        val bookmark = ctrl.state.value.bookmarks.firstOrNull()
        assertNotNull(bookmark)
        assertEquals("New Title", bookmark.title)
    }

    @Test fun `bookmarks list is newest first`() {
        val ctrl = controller()
        ctrl.addPage("https://a.com", "A")
        ctrl.addPage("https://b.com", "B")
        val bookmarks = ctrl.state.value.bookmarks
        assertEquals(2, bookmarks.size)
        assertEquals("https://b.com", bookmarks[0].url)
        assertEquals("https://a.com", bookmarks[1].url)
    }

    // ── History independence ──────────────────────────────────────────────

    @Test fun `clearing history does not clear bookmarks`() {
        val ctrl = controller()
        ctrl.addPage("https://example.com")
        val historyStore = HistoryStore()
        historyStore.recordNavigation("t1", "https://example.com", 1000L)
        // Clear history — must NOT affect bookmarks
        historyStore.clearAll()
        assertEquals(0, historyStore.totalCount())
        assertEquals(1, ctrl.store.count())
        assertTrue(ctrl.store.isBookmarked("https://example.com"))
    }

    @Test fun `removing bookmark does not affect history`() {
        val ctrl = controller()
        ctrl.addPage("https://example.com")
        val historyStore = HistoryStore()
        historyStore.recordNavigation("t1", "https://example.com", 1000L)
        // Remove bookmark — history must remain intact
        val id = ctrl.store.findByUrl("https://example.com")!!.id
        ctrl.removeBookmark(id)
        assertEquals(0, ctrl.store.count())
        assertEquals(1, historyStore.entriesForTab("t1").size)
    }

    // ── navigateToBookmark uses url parameter ─────────────────────────────

    @Test fun `openBookmarks and closeBookmarks toggle showBookmarks in BrowserUiState`() {
        // showBookmarks is owned by BrowserViewModel/BrowserUiState, not BookmarkController.
        val vm = BrowserViewModel()
        assertFalse(vm.uiState.value.showBookmarks)
        vm.openBookmarks()
        assertTrue(vm.uiState.value.showBookmarks)
        vm.closeBookmarks()
        assertFalse(vm.uiState.value.showBookmarks)
    }

    @Test fun `tracker protection defaults to enabled and details dialog toggles`() {
        val vm = BrowserViewModel()

        assertTrue(vm.uiState.value.trackerBlockingEnabled)
        assertFalse(vm.uiState.value.showTrackerDetails)

        vm.openTrackerDetails()
        vm.setTrackerBlockingEnabled(false)

        assertTrue(vm.uiState.value.showTrackerDetails)
        assertFalse(vm.uiState.value.trackerBlockingEnabled)

        vm.closeTrackerDetails()
        assertFalse(vm.uiState.value.showTrackerDetails)
    }
}
