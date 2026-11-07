package dev.fetchx.app.service

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class DownloadNetworkPolicyTest {
    @Test
    fun `wifi-only transfer decision requires wifi`() {
        assertTrue(canDownload(wifiOnly = false, hasWifi = false))
        assertTrue(canDownload(wifiOnly = true, hasWifi = true))
        assertFalse(canDownload(wifiOnly = true, hasWifi = false))
    }
}
