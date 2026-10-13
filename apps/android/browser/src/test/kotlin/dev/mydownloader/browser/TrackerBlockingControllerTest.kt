package dev.fetchx.browser

import org.mozilla.geckoview.ContentBlocking
import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

class TrackerBlockingControllerTest {

    @Test
    fun `blocking is enabled by default and tab stats begin empty`() {
        val controller = TrackerBlockingController()

        assertTrue(controller.state.value.enabled)
        assertTrue(controller.state.value.tabStats.isEmpty())
    }

    @Test
    fun `records GeckoView blocked request count and hostname only`() {
        val controller = TrackerBlockingController()
        controller.registerTab("tab-1")

        assertTrue(
            controller.recordBlocked(
                "tab-1",
                "https://Analytics.Example.test/pixel?user=private-value",
            ),
        )

        val stats = controller.state.value.tabStats.getValue("tab-1")
        assertEquals(1L, stats.blockedRequests)
        assertEquals(listOf("analytics.example.test"), stats.blockedHosts)
    }

    @Test
    fun `GeckoView event integration records only blocking anti-tracking events`() {
        val controller = TrackerBlockingController()
        controller.registerTab("tab-1")

        assertFalse(
            recordTrackerBlockedEvent(
                controller,
                "tab-1",
                isBlocking = true,
                antiTrackingCategory = ContentBlocking.AntiTracking.FINGERPRINTING,
                requestUri = "https://tracker.example.test/fingerprinter",
            ),
        )
        assertFalse(
            recordTrackerBlockedEvent(
                controller,
                "tab-1",
                isBlocking = false,
                antiTrackingCategory = ContentBlocking.AntiTracking.AD,
                requestUri = "https://tracker.example.test/",
            ),
        )
        assertFalse(
            recordTrackerBlockedEvent(
                controller,
                "tab-1",
                isBlocking = true,
                antiTrackingCategory = 0,
                requestUri = "https://tracker.example.test/",
            ),
        )
        assertTrue(
            recordTrackerBlockedEvent(
                controller,
                "tab-1",
                isBlocking = true,
                antiTrackingCategory = ContentBlocking.AntiTracking.AD,
                requestUri = "https://tracker.example.test/path?private=value",
            ),
        )

        val stats = controller.state.value.tabStats.getValue("tab-1")
        assertEquals(1L, stats.blockedRequests)
        assertEquals(listOf("tracker.example.test"), stats.blockedHosts)
    }

    @Test
    fun `stores unique blocked hosts in arrival order`() {
        val controller = TrackerBlockingController()
        controller.registerTab("tab-1")

        controller.recordBlocked("tab-1", "https://tracker.example.test/a?secret=1")
        controller.recordBlocked("tab-1", "https://tracker.example.test/b?secret=2")
        controller.recordBlocked("tab-1", "https://pixel.example.test/")

        val stats = controller.state.value.tabStats.getValue("tab-1")
        assertEquals(3L, stats.blockedRequests)
        assertEquals(
            listOf("tracker.example.test", "pixel.example.test"),
            stats.blockedHosts,
        )
    }

    @Test
    fun `invalid request uri still contributes count without retaining request data`() {
        val controller = TrackerBlockingController()
        controller.registerTab("tab-1")

        assertTrue(controller.recordBlocked("tab-1", "not a url?secret=value"))

        val stats = controller.state.value.tabStats.getValue("tab-1")
        assertEquals(1L, stats.blockedRequests)
        assertTrue(stats.blockedHosts.isEmpty())
    }

    @Test
    fun `disabled protection ignores late blocked callbacks`() {
        val controller = TrackerBlockingController()
        controller.registerTab("tab-1")
        controller.setEnabled(false)

        assertFalse(controller.recordBlocked("tab-1", "https://tracker.example.test/"))
        assertFalse(controller.state.value.enabled)
        assertTrue(controller.state.value.tabStats.isEmpty())
    }

    @Test
    fun `enable and disable changes state without clearing accumulated stats`() {
        val controller = TrackerBlockingController()
        controller.registerTab("tab-1")
        controller.recordBlocked("tab-1", "https://tracker.example.test/")

        controller.setEnabled(false)
        controller.setEnabled(true)

        assertTrue(controller.state.value.enabled)
        assertEquals(1L, controller.state.value.tabStats.getValue("tab-1").blockedRequests)
    }

    @Test
    fun `tab statistics are isolated and removed when tab closes`() {
        val controller = TrackerBlockingController()
        controller.registerTab("tab-1")
        controller.registerTab("tab-2")
        controller.recordBlocked("tab-1", "https://a.example.test/")
        controller.recordBlocked("tab-2", "https://b.example.test/")
        controller.recordBlocked("tab-2", "https://c.example.test/")

        assertEquals(1L, controller.state.value.tabStats.getValue("tab-1").blockedRequests)
        assertEquals(2L, controller.state.value.tabStats.getValue("tab-2").blockedRequests)

        controller.removeTab("tab-1")

        assertNull(controller.state.value.tabStats["tab-1"])
        assertEquals(2L, controller.state.value.tabStats.getValue("tab-2").blockedRequests)
        assertFalse(controller.recordBlocked("tab-1", "https://late.example.test/"))
        assertEquals(2L, controller.state.value.tabStats.getValue("tab-2").blockedRequests)

        controller.registerTab("tab-3")
        assertNull(controller.state.value.tabStats["tab-3"])
        assertTrue(controller.recordBlocked("tab-3", "https://new.example.test/"))
        assertEquals(1L, controller.state.value.tabStats.getValue("tab-3").blockedRequests)
    }

    @Test
    fun `host details remain bounded for long-running tabs`() {
        val controller = TrackerBlockingController()
        controller.registerTab("tab-1")

        repeat(100) { index ->
            controller.recordBlocked("tab-1", "https://tracker$index.example.test/")
        }

        val stats = controller.state.value.tabStats.getValue("tab-1")
        assertEquals(100L, stats.blockedRequests)
        assertEquals(20, stats.blockedHosts.size)
        assertEquals("tracker80.example.test", stats.blockedHosts.first())
        assertEquals("tracker99.example.test", stats.blockedHosts.last())
    }

    @Test
    fun `clear tabs removes all stats but preserves global enabled setting`() {
        val controller = TrackerBlockingController()
        controller.registerTab("tab-1")
        controller.recordBlocked("tab-1", "https://tracker.example.test/")
        controller.setEnabled(false)

        controller.clearTabs()

        assertFalse(controller.state.value.enabled)
        assertTrue(controller.state.value.tabStats.isEmpty())
    }

    @Test
    fun `clear tabs unregisters tabs that have not received any events`() {
        val controller = TrackerBlockingController()
        controller.registerTab("tab-1")

        controller.clearTabs()

        assertFalse(controller.recordBlocked("tab-1", "https://late.example.test/"))
        assertTrue(controller.state.value.tabStats.isEmpty())
    }

    @Test
    fun `blocked request count saturates instead of overflowing`() {
        assertEquals(Long.MAX_VALUE, incrementSaturatedCount(Long.MAX_VALUE))
        assertEquals(Long.MAX_VALUE, incrementSaturatedCount(Long.MAX_VALUE - 1))
        assertEquals(1L, incrementSaturatedCount(0L))
    }

    @Test
    fun `tracker toggle maps to GeckoView native anti-tracking categories`() {
        assertEquals(
            ContentBlocking.AntiTracking.AD or
                ContentBlocking.AntiTracking.ANALYTIC or
                ContentBlocking.AntiTracking.SOCIAL,
            trackerAntiTrackingCategories(true),
        )
        assertEquals(
            ContentBlocking.AntiTracking.NONE,
            trackerAntiTrackingCategories(false),
        )
    }

    @Test
    fun `state changes notify observer`() {
        val controller = TrackerBlockingController()
        controller.registerTab("tab-1")
        var notifications = 0
        controller.onStateChanged = { notifications += 1 }

        controller.recordBlocked("tab-1", "https://tracker.example.test/")
        controller.setEnabled(false)
        controller.setEnabled(false)

        assertEquals(2, notifications)
    }

    @Test
    fun `late callback after tab removal cannot recreate its stats`() {
        val controller = TrackerBlockingController()
        controller.registerTab("tab-1")
        controller.recordBlocked("tab-1", "https://tracker.example.test/")

        controller.removeTab("tab-1")

        assertFalse(controller.recordBlocked("tab-1", "https://late.example.test/"))
        assertNull(controller.state.value.tabStats["tab-1"])
    }

    @Test
    fun `hostname extraction normalizes case and trailing dot for http and https`() {
        assertEquals("tracker.example.test", normalizedHost("http://TRACKER.example.test./pixel"))
        assertEquals("analytics.example.test", normalizedHost("https://Analytics.Example.test/"))
    }

    @Test
    fun `hostname extraction rejects malformed urls`() {
        assertNull(normalizedHost("not a url?secret=value"))
        assertNull(normalizedHost("file:///private/path"))
        assertNull(normalizedHost("ftp://tracker.example.test/resource"))
    }
}
