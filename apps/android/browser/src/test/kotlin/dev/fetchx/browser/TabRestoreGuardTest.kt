package dev.fetchx.browser

import dev.fetchx.browser.TabRestoreGuard
import dev.fetchx.browser.TabRestoreNavigationGate
import dev.fetchx.browser.TabSnapshotWriteGate
import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class TabRestoreGuardTest {
    @Test
    fun `restore result is rejected after a browser state change`() {
        val guard = TabRestoreGuard()
        val restoreVersion = guard.version()

        guard.markStateChanged()

        assertFalse(guard.canApply(restoreVersion))
    }

    @Test
    fun `restore result is rejected after disposal`() {
        val guard = TabRestoreGuard()
        val restoreVersion = guard.version()

        guard.dispose()

        assertFalse(guard.canApply(restoreVersion))
        assertTrue(guard.isDisposed())
    }

    @Test
    fun `user snapshot is held until the restore read completes`() {
        val gate = TabSnapshotWriteGate()
        val persisted = mutableListOf<Pair<List<String>, String?>>()
        gate.beginRestore()

        gate.submit(listOf(TabState("first")), "first") { tabs, activeId ->
            persisted.add(tabs.map { it.id } to activeId)
        }
        gate.submit(listOf(TabState("first"), TabState("new")), "new") { tabs, activeId ->
            persisted.add(tabs.map { it.id } to activeId)
        }

        assertTrue(persisted.isEmpty())
        gate.completeRestore { tabs, activeId ->
            persisted.add(tabs.map { it.id } to activeId)
        }

        val expected: List<Pair<List<String>, String?>> =
            listOf(listOf("first", "new") to "new")
        assertEquals(expected, persisted)
    }

    @Test
    fun `tab creation during restore rejects stale tabs and persists the new identity`() {
        val guard = TabRestoreGuard()
        val gate = TabSnapshotWriteGate()
        val restoreVersion = guard.version()
        val persisted = mutableListOf<Pair<List<String>, String?>>()
        gate.beginRestore()

        guard.markStateChanged()
        gate.submit(listOf(TabState("created-during-restore")), "created-during-restore") { tabs, activeId ->
            persisted.add(tabs.map { it.id } to activeId)
        }

        assertFalse(guard.canApply(restoreVersion))
        assertTrue(persisted.isEmpty())
        gate.completeRestore { tabs, activeId ->
            persisted.add(tabs.map { it.id } to activeId)
        }

        val expected: List<Pair<List<String>, String?>> =
            listOf(listOf("created-during-restore") to "created-during-restore")
        assertEquals(expected, persisted)
    }

    @Test
    fun `dispose drops a deferred snapshot`() {
        val gate = TabSnapshotWriteGate()
        var persisted = false
        gate.beginRestore()
        gate.submit(listOf(TabState("tab")), "tab") { _, _ -> persisted = true }

        gate.dispose()
        gate.completeRestore { _, _ -> persisted = true }

        assertFalse(persisted)
    }

    @Test
    fun `navigation requested during restore is replayed after restore`() {
        val gate = TabRestoreNavigationGate()
        gate.beginRestore()

        assertTrue(gate.defer("https://first.example"))
        assertTrue(gate.defer("https://latest.example"))

        assertEquals("https://latest.example", gate.completeRestore())
        assertFalse(gate.defer("https://after-restore.example"))
    }

    @Test
    fun `disposal drops navigation requested during restore`() {
        val gate = TabRestoreNavigationGate()
        gate.beginRestore()
        gate.defer("https://discarded.example")

        gate.dispose()

        assertEquals(null, gate.completeRestore())
        assertFalse(gate.defer("https://after-disposal.example"))
    }
}
