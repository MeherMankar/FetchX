package dev.fetchx.app.service

import android.app.Service
import org.junit.Assert.assertEquals
import org.junit.Test

class DownloadForegroundServiceTest {
    @Test
    fun `service restart is sticky when bridge can restore persisted downloads`() {
        assertEquals(
            Service.START_STICKY,
            downloadServiceRestartMode(repositoryAvailable = true),
        )
    }

    @Test
    fun `service does not restart when bridge is unavailable`() {
        assertEquals(
            Service.START_NOT_STICKY,
            downloadServiceRestartMode(repositoryAvailable = false),
        )
    }
}
