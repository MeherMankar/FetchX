package dev.fetchx.app.aria2

import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test

class Aria2RemoteTest {
    @Test
    fun `rpc request includes token before task params`() {
        assertEquals(
            """{"jsonrpc":"2.0","id":"fetchx","method":"aria2.addUri","params":["token:secret","https://example.com/file"]}""",
            aria2RpcRequest("aria2.addUri", "secret", listOf("https://example.com/file")),
        )
    }

    @Test
    fun `rpc url must be absolute HTTP endpoint`() {
        assertEquals("http://localhost:6800/jsonrpc", normalizeRpcUrl("http://localhost:6800/jsonrpc/"))
        assertThrows(IllegalArgumentException::class.java) {
            normalizeRpcUrl("ftp://localhost/file")
        }
    }
}
