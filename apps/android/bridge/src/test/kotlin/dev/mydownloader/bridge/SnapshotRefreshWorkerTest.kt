package dev.fetchx.bridge

import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class SnapshotRefreshWorkerTest {

    @Test
    fun `requests during a refresh schedule a follow-up snapshot`() {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
        val firstRefreshStarted = CountDownLatch(1)
        val releaseFirstRefresh = CountDownLatch(1)
        val secondRefreshStarted = CountDownLatch(1)
        val refreshCount = AtomicInteger()
        val worker = SnapshotRefreshWorker(
            scope = scope,
            refresh = {
                if (refreshCount.incrementAndGet() == 1) {
                    firstRefreshStarted.countDown()
                    check(releaseFirstRefresh.await(5, TimeUnit.SECONDS))
                } else {
                    secondRefreshStarted.countDown()
                }
            },
            onError = { throw AssertionError("Snapshot refresh failed", it) },
        )

        try {
            worker.request()
            assertTrue(firstRefreshStarted.await(5, TimeUnit.SECONDS))
            worker.request()
            worker.request()
            releaseFirstRefresh.countDown()

            assertTrue(secondRefreshStarted.await(5, TimeUnit.SECONDS))
            Thread.sleep(100)
            assertEquals(2, refreshCount.get())
        } finally {
            releaseFirstRefresh.countDown()
            scope.cancel()
        }
    }
}
