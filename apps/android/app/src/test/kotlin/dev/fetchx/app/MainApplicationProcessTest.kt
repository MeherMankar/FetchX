package dev.fetchx.app

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class MainApplicationProcessTest {

    @Test
    fun `main process owns the download runtime`() {
        assertTrue(isMainApplicationProcess("dev.fetchx.app.debug", "dev.fetchx.app.debug"))
    }

    @Test
    fun `Gecko child processes do not own the download runtime`() {
        assertFalse(
            isMainApplicationProcess(
                "dev.fetchx.app.debug:tab_disable_art_image_27",
                "dev.fetchx.app.debug",
            ),
        )
        assertFalse(
            isMainApplicationProcess(
                "dev.fetchx.app.debug:gpu_disable_art_image_",
                "dev.fetchx.app.debug",
            ),
        )
    }

    @Test
    fun `unknown process name fails closed`() {
        assertFalse(isMainApplicationProcess(null, "dev.fetchx.app.debug"))
    }
}
