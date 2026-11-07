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

    @Test
    fun `unmetered and roaming restrictions block transfers`() {
        assertFalse(
            isTransferAllowed(
                wifiOnly = false,
                unmeteredOnly = true,
                roamingAllowed = false,
                chargingOnly = false,
                batteryControl = false,
                hasWifi = true,
                hasInternet = true,
                isUnmetered = false,
                isRoaming = false,
                isCharging = false,
                batteryLevel = 80,
                batteryThreshold = 20,
            ),
        )
        assertFalse(
            isTransferAllowed(
                wifiOnly = false,
                unmeteredOnly = false,
                roamingAllowed = false,
                chargingOnly = false,
                batteryControl = false,
                hasWifi = false,
                hasInternet = true,
                isUnmetered = true,
                isRoaming = true,
                isCharging = false,
                batteryLevel = 80,
                batteryThreshold = 20,
            ),
        )
    }

    @Test
    fun `charging and battery controls block transfers`() {
        assertFalse(
            isTransferAllowed(
                wifiOnly = false,
                unmeteredOnly = false,
                roamingAllowed = true,
                chargingOnly = true,
                batteryControl = true,
                hasWifi = true,
                hasInternet = true,
                isUnmetered = true,
                isRoaming = false,
                isCharging = false,
                batteryLevel = 80,
                batteryThreshold = 20,
            ),
        )
        assertFalse(
            isTransferAllowed(
                wifiOnly = false,
                unmeteredOnly = false,
                roamingAllowed = true,
                chargingOnly = false,
                batteryControl = true,
                hasWifi = true,
                hasInternet = true,
                isUnmetered = true,
                isRoaming = false,
                isCharging = true,
                batteryLevel = 15,
                batteryThreshold = 20,
            ),
        )
    }
}
