package dev.fetchx.bridge

import dev.fetchx.bridge.uniffi.DownloadStatusDto
import java.io.File
import java.security.MessageDigest
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class PublicDownloadStorageRecoveryTest {

    @get:Rule
    val temporaryFolder = TemporaryFolder()

    @Test
    fun `insert failure retains staging and reservation for an idempotent retry`() {
        val fixture = Fixture()
        val staging = fixture.stage("file.txt", "download".toByteArray())
        fixture.mediaStore.failNextInsert = true

        expectFailure { fixture.storage.publishCompletedReference(staging) }

        assertTrue(File(staging).isFile)
        assertTrue(fixture.state.entries().keys.any { it.startsWith("reservation_") })
        assertTrue(fixture.mediaStore.rows.isEmpty())
        assertNull(fixture.storage.publishCompletedReference(staging))

        assertNotNull(fixture.storage.publishCompletedReference(staging, forceRetry = true))
        assertEquals(1, fixture.mediaStore.rows.size)
        assertFalse(fixture.mediaStore.rows.single().isPending)
    }

    @Test
    fun `reservation persistence failure does not insert a MediaStore row`() {
        val fixture = Fixture()
        val staging = fixture.stage("file.txt", "download".toByteArray())
        fixture.state.failNextReservationUpdate = true

        expectFailure { fixture.storage.publishCompletedReference(staging) }

        assertTrue(File(staging).isFile)
        assertTrue(fixture.mediaStore.rows.isEmpty())
        assertTrue(fixture.state.entries().keys.none { it.startsWith("published_") })
    }

    @Test
    fun `write failure removes incomplete row and retry reuses reserved name`() {
        val fixture = Fixture()
        val staging = fixture.stage("file.txt", "complete-content".toByteArray())
        fixture.mediaStore.failNextWrite = true

        expectFailure { fixture.storage.publishCompletedReference(staging) }

        assertTrue(File(staging).isFile)
        assertTrue(fixture.mediaStore.rows.isEmpty())
        val reservedName = fixture.reservedNames().single()

        fixture.storage.publishCompletedReference(staging, forceRetry = true)

        assertEquals(listOf(reservedName), fixture.mediaStore.rows.map { it.displayName })
        assertFalse(fixture.mediaStore.rows.single().isPending)
    }

    @Test
    fun `finalization failure cleans pending row before retry`() {
        val fixture = Fixture()
        val staging = fixture.stage("file.txt", "complete-content".toByteArray())
        fixture.mediaStore.failNextFinalize = true

        expectFailure { fixture.storage.publishCompletedReference(staging) }

        assertTrue(File(staging).isFile)
        assertTrue(fixture.mediaStore.rows.isEmpty())

        fixture.storage.publishCompletedReference(staging, forceRetry = true)

        assertEquals(1, fixture.mediaStore.rows.size)
        assertFalse(fixture.mediaStore.rows.single().isPending)
    }

    @Test
    fun `failed publication-state update reconciles finalized row after restart`() {
        val fixture = Fixture()
        val staging = fixture.stage("file.txt", "complete-content".toByteArray())
        fixture.state.failNextPublishedUpdate = true
        fixture.mediaStore.failNextDelete = true

        expectFailure { fixture.storage.publishCompletedReference(staging) }

        assertTrue(File(staging).isFile)
        assertEquals(1, fixture.mediaStore.rows.size)
        assertFalse(fixture.mediaStore.rows.single().isPending)

        val restartedStorage = fixture.newStorage()
        val recoveredUri = restartedStorage.publishCompletedReference(staging)

        assertNotNull(recoveredUri)
        assertEquals(1, fixture.mediaStore.rows.size)
        assertFalse(fixture.mediaStore.rows.single().isPending)
        assertFalse(File(staging).exists())
        assertTrue(fixture.state.entries().keys.any { it.startsWith("published_") })
    }

    @Test
    fun `completed MediaStore row without a pending URI is recovered from its reservation`() {
        val fixture = Fixture()
        val contents = "complete-content".toByteArray()
        val staging = fixture.stage("file.txt", contents)
        val reservation = PublicDownloadReservation("file.txt", contents.size.toLong(), hash(contents))
        fixture.state.update(
            mapOf("reservation_${hash(staging)}" to encodePublicDownloadReservation(reservation)),
        )
        val uri = fixture.mediaStore.insertPending("file.txt", "text/plain", "Download/FetchX/")
        fixture.mediaStore.rows.single().bytes = contents
        fixture.mediaStore.publish(uri)

        val recoveredUri = fixture.newStorage().publishCompletedReference(staging)

        assertEquals(uri, recoveredUri)
        assertEquals(1, fixture.mediaStore.rows.size)
        assertFalse(fixture.mediaStore.rows.single().isPending)
        assertTrue(fixture.state.entries().keys.any { it.startsWith("published_") })
    }

    @Test
    fun `inserted pending row without a persisted URI is safely recovered`() {
        val fixture = Fixture()
        val staging = fixture.stage("file.txt", "complete-content".toByteArray())
        fixture.state.failNextPendingUpdate = true
        fixture.mediaStore.failNextDelete = true

        expectFailure { fixture.storage.publishCompletedReference(staging) }

        assertEquals(1, fixture.mediaStore.rows.size)
        assertTrue(fixture.mediaStore.rows.single().isPending)
        val restartedStorage = fixture.newStorage()

        restartedStorage.reconcileOrphanedPublications(setOf(staging))
        restartedStorage.publishCompletedReference(staging)

        assertEquals(1, fixture.mediaStore.rows.size)
        assertFalse(fixture.mediaStore.rows.single().isPending)
        assertEquals("complete-content", String(fixture.mediaStore.rows.single().bytes))
    }

    @Test
    fun `repeated publication uses one item and new downloads get collision-safe names`() {
        val fixture = Fixture()
        val stagedFiles = listOf(
            fixture.stage("file.txt", "first".toByteArray()),
            fixture.stage("file.txt", "second".toByteArray()),
            fixture.stage("file.txt", "third".toByteArray()),
        )

        stagedFiles.forEach { fixture.storage.publishCompletedReference(it) }
        stagedFiles.forEach { fixture.storage.publishCompletedReference(it, forceRetry = true) }
        val restartedStorage = fixture.newStorage()
        stagedFiles.forEach { restartedStorage.publishCompletedReference(it) }

        assertEquals(
            listOf("file.txt", "file (1).txt", "file (2).txt"),
            fixture.mediaStore.rows.map { it.displayName },
        )
        assertTrue(fixture.mediaStore.rows.none { it.isPending })
    }

    @Test
    fun `staging path avoids existing HTTP partial artifacts`() {
        val fixture = Fixture()
        File(fixture.stagingDirectory, "file.txt.part").writeBytes(byteArrayOf(1, 2, 3))

        val staging = fixture.storage.stageDestination(
            File(fixture.publicDirectory, "file.txt").absolutePath,
        )

        assertEquals(File(fixture.stagingDirectory, "file (1).txt").absolutePath, staging)
    }

    @Test
    fun `orphaned owned pending entries are removed without touching completed rows`() {
        val fixture = Fixture()
        val staleUri = fixture.mediaStore.insertPending("stale.txt", "text/plain", "Download/FetchX/")
        fixture.state.update(
            mapOf(
                "pending_${hash("removed-staging-path")}" to staleUri,
                "reservation_${hash("removed-staging-path")}" to
                    encodePublicDownloadReservation(
                        PublicDownloadReservation("stale.txt", 0, hash(byteArrayOf())),
                    ),
            ),
        )
        val completedUri = fixture.mediaStore.insertPending("keep.txt", "text/plain", "Download/FetchX/")
        fixture.mediaStore.publish(completedUri)

        fixture.storage.reconcileOrphanedPublications(emptySet())

        assertEquals(listOf("keep.txt"), fixture.mediaStore.rows.map { it.displayName })
        assertTrue(fixture.state.entries().keys.none { it.startsWith("pending_") || it.startsWith("reservation_") })
    }

    @Test
    fun `missing staging and MediaStore data never creates a successful state`() {
        val fixture = Fixture()
        val staging = fixture.stage("file.txt", "download".toByteArray())
        File(staging).delete()
        fixture.state.update(
            mapOf("pending_${hash(staging)}" to "content://media/downloads/removed"),
        )

        fixture.storage.reconcileOrphanedPublications(emptySet())
        expectFailure { fixture.storage.publishCompletedReference(staging) }

        assertTrue(fixture.state.entries().keys.none { it.startsWith("published_") })
        assertTrue(fixture.mediaStore.rows.isEmpty())
    }

    @Test
    fun `cancellation waits for an in-progress publication to leave the shared gate`() {
        val fixture = Fixture()
        val publicationEntered = CountDownLatch(1)
        val cancellationAttempting = CountDownLatch(1)
        val publicationMayFinish = CountDownLatch(1)
        var published = false
        var cancelled = false

        val publisher = Thread {
            fixture.storage.withPublicationLock {
                publicationEntered.countDown()
                check(publicationMayFinish.await(5, TimeUnit.SECONDS))
                published = isCurrentCompletedStagingDestination(
                    DownloadStatusDto.COMPLETED,
                    "staging",
                    "staging",
                )
            }
        }
        val canceller = Thread {
            check(publicationEntered.await(5, TimeUnit.SECONDS))
            cancellationAttempting.countDown()
            fixture.storage.withPublicationLock {
                if (shouldCancelDownload(if (published) DownloadStatusDto.COMPLETED else DownloadStatusDto.QUEUED)) {
                    cancelled = true
                }
            }
        }

        publisher.start()
        canceller.start()
        assertTrue(cancellationAttempting.await(5, TimeUnit.SECONDS))
        publicationMayFinish.countDown()
        publisher.join(5_000)
        canceller.join(5_000)

        assertFalse(publisher.isAlive)
        assertFalse(canceller.isAlive)
        assertTrue(published)
        assertFalse(cancelled)
    }

    @Test
    fun `cancellation that wins the gate prevents later publication`() {
        val fixture = Fixture()
        var cancelled = false
        var published = false

        fixture.storage.withPublicationLock {
            cancelled = shouldCancelDownload(DownloadStatusDto.QUEUED)
        }
        fixture.storage.withPublicationLock {
            if (!cancelled) {
                published = isCurrentCompletedStagingDestination(
                    DownloadStatusDto.CANCELLED,
                    "staging",
                    "staging",
                )
            }
        }

        assertTrue(cancelled)
        assertFalse(published)
    }

    @Test
    fun `torrent directories remain outside single-file publication and custom destinations pass through`() {
        val fixture = Fixture()
        val customSafUri = "content://provider/tree/primary%3ADownload"

        assertEquals(customSafUri, fixture.storage.stageDestinationIfPublic(customSafUri))
        assertEquals(
            customSafUri,
            fixture.storage.torrentDirectoryIfPublic(customSafUri),
        )
        assertEquals(
            fixture.storage.privateTorrentDirectoryPath(),
            fixture.storage.torrentDirectoryIfPublic(fixture.storage.publicDirectoryPath()),
        )
        assertTrue(fixture.mediaStore.rows.isEmpty())
    }

    private fun expectFailure(block: () -> Unit) {
        try {
            block()
            fail("Expected the publication attempt to fail.")
        } catch (_: IllegalStateException) {}
    }

    private inner class Fixture {
        private val root = temporaryFolder.newFolder()
        val stagingDirectory = File(root, "private/public-staging").apply { mkdirs() }
        val publicDirectory = File(root, "public/Download/FetchX").apply { mkdirs() }
        val state = FakeState()
        val mediaStore = FakeMediaStore()
        val storage = newStorage()

        fun newStorage() = PublicDownloadStorage(
            stagingDirectory = stagingDirectory,
            publicDirectory = publicDirectory,
            mediaStore = mediaStore,
            state = state,
            appPackageName = "dev.fetchx.test",
        )

        fun stage(name: String, bytes: ByteArray): String {
            val destination = File(publicDirectory, name).absolutePath
            val path = storage.stageDestination(destination)
            File(path).writeBytes(bytes)
            return path
        }

        fun reservedNames(): List<String> =
            state.entries()
                .filterKeys { it.startsWith("reservation_") }
                .values
                .map { decodePublicDownloadReservation(it).name }
    }

    private class FakeState : PublicationStateStore {
        private val values = mutableMapOf<String, String>()
        var failNextReservationUpdate = false
        var failNextPendingUpdate = false
        var failNextPublishedUpdate = false

        override fun get(key: String): String? = values[key]

        override fun contains(key: String): Boolean = values.containsKey(key)

        override fun entries(): Map<String, String> = values.toMap()

        override fun update(changes: Map<String, String?>): Boolean {
            if (failNextReservationUpdate && changes.keys.any { it.startsWith("reservation_") }) {
                failNextReservationUpdate = false
                return false
            }
            if (failNextPendingUpdate && changes.keys.any { it.startsWith("pending_") }) {
                failNextPendingUpdate = false
                return false
            }
            if (failNextPublishedUpdate && changes.keys.any { it.startsWith("published_") }) {
                failNextPublishedUpdate = false
                return false
            }
            changes.forEach { (key, value) ->
                if (value == null) values.remove(key) else values[key] = value
            }
            return true
        }
    }

    private class FakeMediaStore : PublicDownloadMediaStore {
        val rows = mutableListOf<FakeRow>()
        var failNextInsert = false
        var failNextWrite = false
        var failNextFinalize = false
        var failNextDelete = false
        private var nextId = 1

        override fun queryRows(): List<PublicMediaRow> = rows.map(FakeRow::toPublic)

        override fun find(uri: String): PublicMediaRow? = rows.firstOrNull { it.uri == uri }?.toPublic()

        override fun insertPending(displayName: String, mimeType: String, relativePath: String): String {
            if (failNextInsert) {
                failNextInsert = false
                throw IllegalStateException("Injected insertion failure.")
            }
            val uri = "content://media/downloads/${nextId++}"
            rows += FakeRow(uri, displayName, relativePath, byteArrayOf(), true, "dev.fetchx.test")
            return uri
        }

        override fun write(uri: String, source: File): Long {
            val row = rows.single { it.uri == uri }
            val contents = source.readBytes()
            if (failNextWrite) {
                failNextWrite = false
                row.bytes = contents.copyOf(contents.size / 2)
                throw IllegalStateException("Injected provider write failure.")
            }
            row.bytes = contents
            return contents.size.toLong()
        }

        override fun sha256(uri: String): String =
            hash(rows.single { it.uri == uri }.bytes)

        override fun publish(uri: String): Boolean {
            if (failNextFinalize) {
                failNextFinalize = false
                return false
            }
            rows.single { it.uri == uri }.isPending = false
            return true
        }

        override fun delete(uri: String): Boolean {
            if (failNextDelete) {
                failNextDelete = false
                return false
            }
            return rows.removeAll { it.uri == uri }
        }

        data class FakeRow(
            val uri: String,
            val displayName: String,
            val relativePath: String,
            var bytes: ByteArray,
            var isPending: Boolean,
            val ownerPackageName: String?,
        ) {
            val size: Long get() = bytes.size.toLong()

            fun toPublic() = PublicMediaRow(
                uri,
                displayName,
                relativePath,
                size,
                isPending,
                ownerPackageName,
            )
        }
    }

    private companion object {
        fun hash(value: String): String = hash(value.toByteArray())

        fun hash(bytes: ByteArray): String =
            MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }
    }
}
