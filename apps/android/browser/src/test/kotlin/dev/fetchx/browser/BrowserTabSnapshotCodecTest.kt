package dev.fetchx.browser

import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertNull

class BrowserTabSnapshotCodecTest {
    @Test
    fun `versioned snapshot round trips tabs and active ID`() {
        val snapshot = BrowserTabSnapshot(
            generation = 4,
            tabs = listOf(
                TabState("tab-1", "https://one.example", "One", spaceId = "default"),
                TabState("tab-2", "https://two.example", "Two", spaceId = "work", desktopSiteEnabled = true),
            ),
            activeTabId = "tab-2",
        )

        assertEquals(snapshot, BrowserTabSnapshotCodec.decode(BrowserTabSnapshotCodec.encode(snapshot)))
    }

    @Test
    fun `checksum rejects modified snapshot bytes`() {
        val encoded = BrowserTabSnapshotCodec.encode(
            BrowserTabSnapshot(1, listOf(TabState("tab-1")), "tab-1"),
        )
        val corrupted = encoded.copyOf().also { bytes ->
            bytes[bytes.lastIndex] = if (bytes.last() == '0'.code.toByte()) '1'.code.toByte() else '0'.code.toByte()
        }

        assertNull(BrowserTabSnapshotCodec.decode(corrupted))
    }

    @Test
    fun `snapshot rejects duplicate tabs and active IDs outside the snapshot`() {
        val duplicateTabs = listOf(TabState("tab-1"), TabState("tab-1", "https://duplicate.example"))
        val duplicateSnapshot = BrowserTabSnapshot(1, duplicateTabs, "tab-1")

        assertFailsWith<IllegalArgumentException> { BrowserTabSnapshotCodec.encode(duplicateSnapshot) }
        assertFailsWith<IllegalArgumentException> {
            BrowserTabSnapshotCodec.encode(BrowserTabSnapshot(1, listOf(TabState("tab-1")), "missing"))
        }
    }
}
